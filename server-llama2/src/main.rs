use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::{extract::State, routing::post, Json, Router};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::llama::{Cache as LlamaCache, Config, Llama, LlamaConfig};
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;
use tokio::net::TcpListener;

use axum::extract::Query;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;
use tokio_stream::wrappers::ReceiverStream;

use candle_core::{Error as CandleError, IndexOp, Result as CandleResult};
use tower_http::cors::{Any, CorsLayer};

mod db;
use crate::db::{load_all_history, save_chat_turn, SessionWithMessages};
use db::{init_db, DbPool};

#[derive(Clone)]
struct AppState {
    model: Arc<Llama>,
    config: Config,
    dtype: DType,
    device: Device,
    tokenizer: Arc<Tokenizer>,
    db_pool: DbPool,
}

#[derive(Deserialize)]
struct ChatRequest {
    prompt: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

#[derive(Deserialize, Clone)]
struct ChatStreamQuery {
    pub session_id: String,
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
    let db_pool = init_db().await?;
    let state = load_tinyllama_state(db_pool)?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/chat", post(chat_handler))
        .route("/chat/stream", axum::routing::get(chat_stream_handler))
        .route("/history", axum::routing::get(history_handler))
        .layer(cors)
        .with_state(state);

    let addr: std::net::SocketAddr = "0.0.0.0:8000".parse().unwrap();
    println!("🚀 Candle TinyLlama server running on http://{addr}");

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

async fn chat_stream_handler(
    State(state): State<AppState>,
    Query(params): Query<ChatStreamQuery>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    println!(
        "[TinyLlama] Received frontend request. Prompt: {}",
        params.prompt
    );

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);

    let state_for_gen = state.clone();
    let params_for_gen = params.clone();
    let state_for_db = state.clone();
    let params_for_db = params.clone();

    let handle =
        spawn_blocking(move || run_streaming_generation(state_for_gen, params_for_gen, tx));

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
                    eprintln!("[TinyLlama] Failed to save chat turn: {e}");
                } else {
                    println!("[TinyLlama] Chat turn saved to DB.");
                }
            }
            Ok(Err(e)) => {
                eprintln!("[TinyLlama] Generation error: {e}");
            }
            Err(join_err) => {
                eprintln!("[TinyLlama] Join error in spawn_blocking: {join_err}");
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn run_streaming_generation(
    state: AppState,
    params: ChatStreamQuery,
    tx: mpsc::Sender<Result<Event, Infallible>>,
) -> anyhow::Result<String> {
    let model = Arc::clone(&state.model);
    let tokenizer = Arc::clone(&state.tokenizer);
    let device = state.device.clone();

    println!("--> [TinyLlama] Creating KV cache...");
    let mut cache = LlamaCache::new(true, state.dtype, &state.config, &device)?;

    println!("--> [TinyLlama] Encoding prompt...");
    let encoding = tokenizer
        .encode(params.prompt.clone(), true)
        .map_err(candle_core::Error::msg)?;
    let mut tokens: Vec<u32> = encoding.get_ids().to_vec();

    let eos_token = tokenizer.get_vocab(true).get("</s>").copied().unwrap_or(2);

    let mut start_pos: usize = 0;
    let seed = 42;
    let temperature = Some(0.7);
    let top_p = Some(0.9);
    let mut logits_processor = LogitsProcessor::new(seed, temperature, top_p);

    let mut prev_text_len = 0usize;
    let mut final_answer = String::new();

    let max_steps = params.max_tokens.min(256);
    println!("--> [TinyLlama] Entering generation loop (max_steps = {max_steps})...");

    for step in 0..max_steps {
        let context_size = if step > 0 { 1 } else { tokens.len() };
        let start_at = tokens.len().saturating_sub(context_size);
        let ctx = &tokens[start_at..];

        let input = Tensor::new(ctx, &device)?.reshape((1, ctx.len()))?;

        let logits = model.forward(&input, start_pos, &mut cache)?;
        start_pos += ctx.len();

        let logits = logits.i(0)?.to_dtype(DType::F32)?;
        let next_token = logits_processor.sample(&logits)?;
        tokens.push(next_token);

        let full_text = tokenizer
            .decode(&tokens, true)
            .map_err(candle_core::Error::msg)?;

        println!(
            "--> [TinyLlama] step {step}, current text length {}",
            full_text.len()
        );

        if full_text.len() > prev_text_len {
            let new_part = &full_text[prev_text_len..];

            if !new_part.is_empty() {
                final_answer.push_str(new_part);

                let event = Event::default().event("message").data(new_part.to_string());

                if tx.blocking_send(Ok(event)).is_err() {
                    println!("--> [TinyLlama] Client disconnected, stopping generation");
                    break;
                }
                prev_text_len = full_text.len();
            }
        }

        if next_token == eos_token {
            println!("--> [TinyLlama] Hit EOS, stopping generation");
            break;
        }

        if step + 1 == max_steps {
            println!("--> [TinyLlama] Reached max_steps = {max_steps}, stopping generation");
        }
    }

    let _ = tx.blocking_send(Ok(Event::default().event("message").data("[DONE]")));
    println!("--> [TinyLlama] Generation finished, sent [DONE]");

    Ok(final_answer)
}

async fn chat_handler(
    State(_state): State<AppState>,
    Json(_req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, axum::http::StatusCode> {
    Ok(Json(ChatResponse {
        response: "Please use /chat/stream for now.".to_string(),
    }))
}

async fn history_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionWithMessages>>, axum::http::StatusCode> {
    match load_all_history(&state.db_pool).await {
        Ok(history) => Ok(Json(history)),
        Err(e) => {
            eprintln!("[TinyLlama DB] Failed to load history: {}", e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn load_tinyllama_state(db_pool: DbPool) -> Result<AppState> {
    let model_dir = PathBuf::from("models/tinyllama");
    let tokenizer_path = model_dir.join("tokenizer.json");
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer error: {e}"))?;

    let config_bytes = std::fs::read(model_dir.join("config.json"))?;
    let llama_config: LlamaConfig = serde_json::from_slice(&config_bytes)?;

    let config = llama_config.into_config(false);

    let filenames = vec![model_dir.join("model.safetensors")];
    let device = Device::Cpu;
    let dtype = DType::F32;

    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&filenames, dtype, &device)? };
    let model = Llama::load(vb, &config)?;

    Ok(AppState {
        model: Arc::new(model),
        config,
        dtype,
        device,
        tokenizer: Arc::new(tokenizer),
        db_pool,
    })
}
