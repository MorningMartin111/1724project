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

#[derive(Clone)]
struct AppState {
    // Qwen2 uses an internal KV cache, so the model is mutable.
    // We wrap it in Arc<Mutex<..>> so we can use it safely from spawn_blocking.
    model: Arc<Mutex<ModelForCausalLM>>,
    config: QwenConfig,
    dtype: DType,
    device: Device,
    tokenizer: Arc<Tokenizer>,
}

#[derive(Deserialize)]
struct ChatRequest {
    prompt: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

#[derive(Deserialize)]
struct ChatStreamQuery {
    prompt: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
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
    let state = load_qwen_state()?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/chat", post(chat_handler))
        .route("/chat/stream", axum::routing::get(chat_stream_handler))
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
    println!("收到前端请求 (Qwen2)！Prompt: {}", params.prompt);

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);

    // Move state + params into a blocking thread for CPU-bound generation.
    spawn_blocking(move || {
        run_streaming_generation_qwen(state, params, tx);
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn run_streaming_generation_qwen(
    state: AppState,
    params: ChatStreamQuery,
    tx: mpsc::Sender<Result<Event, Infallible>>,
) {
    let model_arc = Arc::clone(&state.model);
    let tokenizer = Arc::clone(&state.tokenizer);
    let device = state.device.clone();

    let result: anyhow::Result<()> = (|| {
        println!("--> [Qwen2] 获取模型锁...");
        let mut model = model_arc
            .lock()
            .map_err(|_| anyhow::anyhow!("failed to lock Qwen2 model"))?;

        // New request → clear cached KV
        model.clear_kv_cache();

        println!("--> [Qwen2] 开始编码 Prompt...");
        let encoding = tokenizer
            .encode(params.prompt.clone(), true)
            .map_err(candle_core::Error::msg)?;
        let mut tokens: Vec<u32> = encoding.get_ids().to_vec();

        // You can change EOS to the proper Qwen2 eos if needed
        let eos_token = tokenizer.get_vocab(true).get("</s>").copied().unwrap_or(2);

        let mut seqlen_offset: usize = 0;
        let mut prev_text_len: usize = 0;

        let seed = 42;
        let temperature = Some(0.7);
        let top_p = Some(0.9);
        let mut logits_processor = LogitsProcessor::new(seed, temperature, top_p);

        println!("--> [Qwen2] 进入生成循环...");
        for step in 0..params.max_tokens {
            // Hard cap to avoid insane values from frontend
            let max_steps = params.max_tokens.min(256);

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

                // ---- Stop conditions ----
                if next_token == eos_token {
                    println!("--> [Qwen2] 命中 EOS, 结束生成");
                    break;
                }
                if step + 1 == max_steps {
                    println!("--> [Qwen2] 达到 max_steps = {max_steps}, 结束生成");
                }

                // ---- Streaming text out ----
                let text = tokenizer
                    .decode(&tokens, true)
                    .map_err(|e| anyhow::anyhow!("tokenizer decode error: {e}"))?;

                let new_text = &text[prev_text_len..];
                prev_text_len = text.len();

                if !new_text.is_empty() {
                    let event = Event::default().data(new_text.to_string());
                    if tx.blocking_send(Ok(event)).is_err() {
                        println!("--> [Qwen2] 客户端已断开, 停止生成");
                        break;
                    }
                }
            }

            // send final DONE event so frontend knows to stop
            let _ = tx.blocking_send(Ok(Event::default()
                .event("message") // keep the same event name as normal tokens
                .data("[DONE]")));
            println!("--> [Qwen2] 发送 DONE 事件, 结束生成");
            break;
        }

        Ok(())
    })();

    if let Err(err) = result {
        println!("!!! [Qwen2] 生成过程出错: {err}");
        let _ = tx.blocking_send(Ok(Event::default()
            .event("message")
            .data(format!("\n[Qwen2 Error: {err}]"))));
    }
}

async fn chat_handler(
    State(_state): State<AppState>,
    Json(_req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, axum::http::StatusCode> {
    Ok(Json(ChatResponse {
        response: "Please use /chat/stream for Qwen2 as well.".to_string(),
    }))
}

fn load_qwen_state() -> Result<AppState> {
    // Adjust this path to wherever you stored qwen2_0_5b_instruct
    let model_dir = PathBuf::from("models/qwen2_0_5b_instruct");
    let tokenizer_path = model_dir.join("tokenizer.json");

    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer error: {e}"))?;

    // Qwen2 uses its own Config struct directly
    let config_bytes = std::fs::read(model_dir.join("config.json"))?;
    let qwen_config: QwenConfig = serde_json::from_slice(&config_bytes)?;

    let filenames = vec![model_dir.join("model.safetensors")];

    let device = Device::Cpu; // change to cuda/metal if you’ve built with those features
    let dtype = DType::F32;

    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&filenames, dtype, &device)? };
    let model = ModelForCausalLM::new(&qwen_config, vb)?;

    Ok(AppState {
        model: Arc::new(Mutex::new(model)),
        config: qwen_config,
        dtype,
        device,
        tokenizer: Arc::new(tokenizer),
    })
}
