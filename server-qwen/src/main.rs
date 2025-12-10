use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::Query;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{extract::State, routing::post, Json, Router};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::qwen2::{Config as QwenConfig, ModelForCausalLM};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tokenizers::Tokenizer;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{Any, CorsLayer};

mod db;

use crate::db::{load_all_history, save_chat_turn, SessionWithMessages};
use db::{init_db, DbPool};

#[derive(Clone)]
struct AppState {
    model: Arc<Mutex<ModelForCausalLM>>,
    config: QwenConfig,
    dtype: DType,
    device: Device,
    tokenizer: Arc<Tokenizer>,
    db_pool: DbPool, // NEW
}

#[derive(Deserialize)]
struct ChatRequest {
    prompt: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

#[derive(Deserialize, Clone)]
struct ChatStreamQuery {
    pub session_id: String, // <-- NEW
    pub prompt: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
}

#[derive(Serialize)]
struct ChatResponse {
    response: String,
}

fn default_max_tokens() -> usize {
    64
}

#[tokio::main]
async fn main() -> Result<()> {
    // let state = load_qwen_state()?;
    let db_pool = init_db().await?;
    let state = load_qwen_state(db_pool)?;
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    // pass pool in
    let app = Router::new()
        .route("/chat", post(chat_handler))
        .route("/chat/stream", axum::routing::get(chat_stream_handler))
        .route("/history", axum::routing::get(history_handler))
        .layer(cors)
        .with_state(state);

    let addr: std::net::SocketAddr = "0.0.0.0:8001".parse().unwrap();
    println!("🚀 Candle Qwen2 0.5B Instruct server running on http://{addr}");

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

async fn chat_stream_handler(
    State(state): State<AppState>,
    Query(params): Query<ChatStreamQuery>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    println!(
        "[Qwen2] Received frontend request. Prompt: {}",
        params.prompt
    );

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);

    // clones for different uses
    let state_for_gen = state.clone();
    let params_for_gen = params.clone();
    let state_for_db = state.clone();
    let params_for_db = params.clone();

    // CPU-bound generation in a blocking thread.
    // IMPORTANT: the closure must *return* the result of run_streaming_generation_qwen
    // so handle.await has type Result<anyhow::Result<String>, JoinError>.
    let handle =
        spawn_blocking(move || run_streaming_generation_qwen(state_for_gen, params_for_gen, tx));

    // async task that waits for generation to finish, then saves to DB
    tokio::spawn(async move {
        match handle.await {
            // handle.await : Result<anyhow::Result<String>, JoinError>
            Ok(Ok(final_answer)) => {
                if let Err(e) = save_chat_turn(
                    &state_for_db.db_pool,
                    &params_for_db.session_id,
                    &params_for_db.prompt,
                    &final_answer,
                )
                .await
                {
                    eprintln!("[Qwen2] Failed to save chat turn: {e}");
                } else {
                    println!("[Qwen2] Chat turn saved to DB.");
                }
            }
            Ok(Err(e)) => {
                eprintln!("[Qwen2] Generation error: {e}");
            }
            Err(join_err) => {
                eprintln!("[Qwen2] Join error in spawn_blocking: {join_err}");
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn run_streaming_generation_qwen(
    state: AppState,
    params: ChatStreamQuery,
    tx: mpsc::Sender<Result<Event, Infallible>>,
) -> anyhow::Result<String> {
    let model_arc = Arc::clone(&state.model);
    let tokenizer = Arc::clone(&state.tokenizer);
    let device = state.device.clone();

    println!("--> [Qwen2] Acquiring model lock...");
    let mut model = model_arc
        .lock()
        .map_err(|_| anyhow::anyhow!("failed to lock Qwen2 model"))?;

    // New request → clear cached KV
    model.clear_kv_cache();

    println!("--> [Qwen2] Encoding prompt...");
    let encoding = tokenizer
        .encode(params.prompt.clone(), true)
        .map_err(candle_core::Error::msg)?;
    let mut tokens: Vec<u32> = encoding.get_ids().to_vec();

    // EOS token (adjust if Qwen2 uses a different one in your tokenizer)
    let eos_token = tokenizer.get_vocab(true).get("</s>").copied().unwrap_or(2);

    let mut seqlen_offset: usize = 0;
    let mut prev_text_len: usize = 0;

    // This will be what you save into the DB as the assistant answer
    let mut final_answer = String::new();

    let seed = 42;
    let temperature = Some(0.7);
    let top_p = Some(0.9);
    let mut logits_processor = LogitsProcessor::new(seed, temperature, top_p);

    // Hard cap to avoid insane values from frontend
    let max_steps = params.max_tokens.min(256);
    println!("--> [Qwen2] Entering generation loop (max_steps = {max_steps})...");

    for step in 0..max_steps {
        let context_size = if step > 0 { 1 } else { tokens.len() };
        let start_at = tokens.len().saturating_sub(context_size);
        let ctx = &tokens[start_at..];

        let input = Tensor::new(ctx, &device)?.reshape((1, ctx.len()))?;

        // Forward pass – Qwen2 returns [batch, seq_len, vocab]
        let logits_all = model.forward(&input, seqlen_offset)?;
        seqlen_offset += ctx.len();

        // dims: [1, seq_len, vocab]
        let dims = logits_all.dims();
        let last_pos = dims[1] - 1; // seq_len - 1

        // Take batch = 0, last sequence position -> [vocab]
        let logits = logits_all
            .i((0, last_pos))? // -> [vocab]
            .to_dtype(DType::F32)?; // logits_processor expects f32

        let next_token = logits_processor.sample(&logits)?;
        tokens.push(next_token);

        // Decode full text and figure out the *new* part
        let text = tokenizer
            .decode(&tokens, true)
            .map_err(|e| anyhow::anyhow!("tokenizer decode error: {e}"))?;

        let new_text = &text[prev_text_len..];
        prev_text_len = text.len();

        if !new_text.is_empty() {
            // append to final answer (for DB)
            final_answer.push_str(new_text);

            // stream to client
            let event = Event::default().data(new_text.to_string());
            if tx.blocking_send(Ok(event)).is_err() {
                println!("--> [Qwen2] Client disconnected, stopping generation");
                break;
            }
        }

        // ---- Stop conditions ----
        if next_token == eos_token {
            println!("--> [Qwen2] Hit EOS, stopping generation");
            break;
        }

        if step + 1 == max_steps {
            println!("--> [Qwen2] Reached max_steps = {max_steps}, stopping generation");
        }
    }

    // send final DONE event so frontend knows to stop
    let _ = tx.blocking_send(Ok(Event::default()
        .event("message") // keep same event name as normal tokens
        .data("[DONE]")));
    println!("--> [Qwen2] Sent DONE event, finishing generation");

    Ok(final_answer)
}

async fn chat_handler(
    State(_state): State<AppState>,
    Json(_req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, axum::http::StatusCode> {
    Ok(Json(ChatResponse {
        response: "Please use /chat/stream for Qwen2 as well.".to_string(),
    }))
}

fn load_qwen_state(db_pool: DbPool) -> Result<AppState> {
    let model_dir = PathBuf::from("models/qwen2_0_5b_instruct");
    let tokenizer_path = model_dir.join("tokenizer.json");

    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer error: {e}"))?;

    let config_bytes = std::fs::read(model_dir.join("config.json"))?;
    let qwen_config: QwenConfig = serde_json::from_slice(&config_bytes)?;

    let filenames = vec![model_dir.join("model.safetensors")];

    let device = Device::Cpu;
    let dtype = DType::F32;

    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&filenames, dtype, &device)? };
    let model = ModelForCausalLM::new(&qwen_config, vb)?;

    Ok(AppState {
        model: Arc::new(Mutex::new(model)),
        config: qwen_config,
        dtype,
        device,
        tokenizer: Arc::new(tokenizer),
        db_pool, // 🔹 store the pool
    })
}

async fn history_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionWithMessages>>, axum::http::StatusCode> {
    match load_all_history(&state.db_pool).await {
        Ok(history) => Ok(Json(history)),
        Err(e) => {
            eprintln!("[DB] Failed to load history: {}", e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
