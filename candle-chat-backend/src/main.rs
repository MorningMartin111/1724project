use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::{extract::State, routing::post, Json, Router};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::LogitsProcessor;
// 修正引用：同时引入 Config
use candle_transformers::models::llama::{Cache as LlamaCache, Llama, LlamaConfig, Config};
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

#[derive(Clone)]
struct AppState {
    model: Arc<Llama>,
    // 关键修正：类型改为 Config，而不是 LlamaConfig
    config: Config,                         
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
    // 1. 加载模型
    let state = load_tinyllama_state()?;

    // 2. 配置 CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 3. 路由
    let app = Router::new()
        .route("/chat", post(chat_handler))
        .route("/chat/stream", axum::routing::get(chat_stream_handler))
        .layer(cors)
        .with_state(state);

    let addr: std::net::SocketAddr = "0.0.0.0:8000".parse().unwrap();
    println!("🚀 Candle TinyLlama server running on http://{addr}");

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

// --- 流式处理 Handler ---
async fn chat_stream_handler(
    State(state): State<AppState>,
    Query(params): Query<ChatStreamQuery>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    // --- 新增日志 ---
    println!("收到前端请求！正在准备生成: {}", params.prompt);
    
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);

    spawn_blocking(move || {
        run_streaming_generation(state, params, tx);
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn run_streaming_generation(
    state: AppState,
    params: ChatStreamQuery,
    tx: mpsc::Sender<Result<Event, Infallible>>,
) {
    let model = Arc::clone(&state.model);
    let tokenizer = Arc::clone(&state.tokenizer);
    let device = state.device.clone();

    let result: anyhow::Result<()> = (|| {
        println!("--> 开始创建 Cache...");
        let mut cache = LlamaCache::new(true, state.dtype, &state.config, &device)?;
        
        println!("--> Cache 创建成功，开始编码 Prompt...");
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

        println!("--> 进入生成循环...");
        for step in 0..params.max_tokens {
            let context_size = if step > 0 { 1 } else { tokens.len() };
            let start_at = tokens.len().saturating_sub(context_size);
            let ctx = &tokens[start_at..];

            let input = Tensor::new(ctx, &device)?.reshape((1, ctx.len()))?;

            let logits = model.forward(&input, start_pos, &mut cache)?;
            start_pos += ctx.len();

            let logits = logits.i(0)?.to_dtype(DType::F32)?;
            let next_token = logits_processor.sample(&logits)?;
            tokens.push(next_token);

            if next_token == eos_token {
                println!("--> 生成结束 (遇到 EOS)");
                break;
            }

            let full_text = tokenizer
                .decode(&tokens, true)
                .map_err(candle_core::Error::msg)?;

            // 打印进度，证明没卡死
            println!("--> 生成第 {} 步: 当前文本长度 {}", step, full_text.len());

            if full_text.len() > prev_text_len {
                let new_part = &full_text[prev_text_len..];
                
                if !new_part.is_empty() {
                    let event = Event::default()
                        .event("message")
                        .data(new_part.to_string());

                    if tx.blocking_send(Ok(event)).is_err() {
                        println!("--> 前端断开了连接");
                        break;
                    }
                    prev_text_len = full_text.len();
                }
            }
        }
        Ok(())
    })();

    if let Err(err) = result {
        println!("!!! 生成过程出错: {}", err);
        let _ = tx.blocking_send(Ok(
            Event::default()
                .event("message")
                .data(format!("\n[Error: {}]", err))
        ));
    }
}

// --- 普通请求 Handler (简化版) ---
async fn chat_handler(
    State(_state): State<AppState>,
    Json(_req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, axum::http::StatusCode> {
    // 为了防止旧代码报错，这里直接返回提示
    Ok(Json(ChatResponse { response: "Please use /chat/stream for now.".to_string() }))
}

fn load_tinyllama_state() -> Result<AppState> {
    let model_dir = PathBuf::from("models/tinyllama");
    let tokenizer_path = model_dir.join("tokenizer.json");
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer error: {e}"))?;

    let config_bytes = std::fs::read(model_dir.join("config.json"))?;
    let llama_config: LlamaConfig = serde_json::from_slice(&config_bytes)?;
    
    // 转换为 Config 类型
    let config = llama_config.into_config(false);

    let filenames = vec![model_dir.join("model.safetensors")];
    let device = Device::Cpu; 
    let dtype = DType::F32; 

    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&filenames, dtype, &device)? };
    let model = Llama::load(vb, &config)?;

    Ok(AppState {
        model: Arc::new(model),
        config,          // 这里传入的是 Config 类型
        dtype,
        device,
        tokenizer: Arc::new(tokenizer),
    })
}