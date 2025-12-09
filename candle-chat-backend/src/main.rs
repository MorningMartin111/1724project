use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::{extract::State, routing::post, Json, Router};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::llama as llama_model;
use llama_model::{Cache as LlamaCache, Llama, LlamaConfig};
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;
use tokio::net::TcpListener;

use axum::extract::Query;
use axum::response::sse::{Event, Sse};
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;
use tokio_stream::wrappers::ReceiverStream;

use candle_core::{Error as CandleError, IndexOp, Result as CandleResult};

// --- 新增：引入 CORS 相关模块 ---
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
struct AppState {
    model: Arc<Llama>,
    cache: Arc<std::sync::Mutex<LlamaCache>>,
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

fn take_last_token_logits(logits: &Tensor) -> CandleResult<Tensor> {
    let dims = logits.dims();
    // println!("logits dims = {:?}", dims); // 注释掉日志减少刷屏

    match &dims[..] {
        [1, seq, _vocab] => {
            if *seq == 0 {
                return Err(CandleError::Msg("empty sequence in logits".into()));
            }
            let last = seq - 1;
            logits.i((0, last, ..))
        }
        [seq, _vocab] => {
            if *seq == 0 {
                return Err(CandleError::Msg("empty sequence in logits (2D)".into()));
            }
            let last = seq - 1;
            logits.i((last, ..))
        }
        other => Err(CandleError::Msg(
            format!("unexpected logits shape: {other:?}").into(),
        )),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1) Load TinyLlama model + tokenizer into memory
    let state = load_tinyllama_state()?;

    // --- 新增：配置 CORS 允许所有来源 ---
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/chat", post(chat_handler))
        .route("/chat/stream", axum::routing::get(chat_stream_handler))
        .layer(cors) // --- 应用 CORS 层 ---
        .with_state(state);

    let addr: std::net::SocketAddr = "0.0.0.0:8000".parse().unwrap();

    println!("🚀 Candle TinyLlama server on http://{addr}/chat");

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

async fn chat_stream_handler(
    State(state): State<AppState>,
    Query(params): Query<ChatStreamQuery>,
) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);

    spawn_blocking(move || {
        run_streaming_generation(state, params, tx);
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream)
}

fn run_streaming_generation(
    state: AppState,
    params: ChatStreamQuery,
    tx: mpsc::Sender<Result<Event, Infallible>>,
) {
    let model = Arc::clone(&state.model);
    let cache_arc = Arc::clone(&state.cache);
    let tokenizer = Arc::clone(&state.tokenizer);
    let device = state.device.clone();

    let result: anyhow::Result<()> = (|| {
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

        for step in 0..params.max_tokens {
            let context_size = if step > 0 { 1 } else { tokens.len() };
            let start_at = tokens.len().saturating_sub(context_size);
            let ctx = &tokens[start_at..];

            let input = Tensor::new(ctx, &device)?.reshape((1, ctx.len()))?;

            let logits = {
                let mut cache = cache_arc.lock().unwrap();
                model.forward(&input, start_pos, &mut cache)?
            };
            start_pos += ctx.len();

            let logits = logits.i(0)?.to_dtype(DType::F32)?;
            let next_token = logits_processor.sample(&logits)?;
            tokens.push(next_token);

            if next_token == eos_token {
                break;
            }

            let full_text = tokenizer
                .decode(&tokens, true)
                .map_err(candle_core::Error::msg)?;
            let new_part = &full_text[prev_text_len..];
            prev_text_len = full_text.len();

            if !new_part.is_empty() {
                if tx
                    .blocking_send(Ok(Event::default().data(new_part.to_string())))
                    .is_err()
                {
                    break;
                }
            }
        }

        Ok(())
    })();

    if let Err(err) = result {
        let _ = tx.blocking_send(Ok(
            Event::default().data(format!("[error] generation failed: {err}"))
        ));
    }
}

async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, axum::http::StatusCode> {
    let mut generator = TextGenerator::new(&state);
    let reply = generator
        .generate(&req.prompt, req.max_tokens)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ChatResponse { response: reply }))
}

fn load_tinyllama_state() -> Result<AppState> {
    let model_dir = PathBuf::from("models/tinyllama");

    let tokenizer_path = model_dir.join("tokenizer.json");
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer error: {e}"))?;

    let config_bytes = std::fs::read(model_dir.join("config.json"))?;
    let llama_config: LlamaConfig = serde_json::from_slice(&config_bytes)?;
    let config = llama_config.into_config(false);

    let filenames = vec![model_dir.join("model.safetensors")];
    let device = Device::Cpu;
    let dtype = DType::F16;

    let cache = LlamaCache::new(true, dtype, &config, &device)?;
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&filenames, dtype, &device)? };
    let model = Llama::load(vb, &config)?;

    Ok(AppState {
        model: Arc::new(model),
        cache: Arc::new(std::sync::Mutex::new(cache)),
        device,
        tokenizer: Arc::new(tokenizer),
    })
}

struct TextGenerator {
    model: Arc<Llama>,
    cache: std::sync::MutexGuard<'static, LlamaCache>,
    device: Device,
    tokenizer: Tokenizer,
    logits_processor: LogitsProcessor,
}

impl TextGenerator {
    fn new(state: &AppState) -> Self {
        let cache_arc = unsafe {
            &*(&*state.cache as *const std::sync::Mutex<LlamaCache>)
        };
        let cache = cache_arc.lock().unwrap();

        let seed = 42;
        let temperature = Some(0.7);
        let top_p = Some(0.9);
        let logits_processor = LogitsProcessor::new(seed, temperature, top_p);

        Self {
            model: Arc::clone(&state.model),
            cache,
            device: state.device.clone(),
            tokenizer: (*state.tokenizer).clone(),
            logits_processor,
        }
    }

    fn generate(&mut self, prompt: &str, max_tokens: usize) -> Result<String> {
        let mut tokens = self
            .tokenizer
            .encode(prompt, true)
            .map_err(candle_core::Error::msg)?
            .get_ids()
            .to_vec();

        let eos_token = self
            .tokenizer
            .get_vocab(true)
            .get("</s>")
            .copied()
            .unwrap_or_else(|| 2);

        let mut generated_text = String::new();
        let mut start_pos = 0usize;

        for step in 0..max_tokens {
            let context_size = if step > 0 { 1 } else { tokens.len() };
            let start_at = tokens.len().saturating_sub(context_size);
            let ctx = &tokens[start_at..];

            let input = Tensor::new(ctx, &self.device)?.reshape((1, ctx.len()))?;
            let logits = self.model.forward(&input, start_pos, &mut self.cache)?;
            start_pos += ctx.len();

            let logits = take_last_token_logits(&logits)?;
            let logits = logits.to_dtype(DType::F32)?;

            let next_token = self.logits_processor.sample(&logits)?;
            tokens.push(next_token);

            if next_token == eos_token {
                break;
            }

            let text = self
                .tokenizer
                .decode(&tokens, true)
                .map_err(candle_core::Error::msg)?;

            generated_text = text;
        }

        Ok(generated_text)
    }
}