# Setup Instructions

Follow these steps to set up and run the TinyLlama Candle Chat Backend.

## 1. Install the Hugging Face Hub Python client
```bash
pip install huggingface_hub
```

## 2. python3 download_tinyllama.py
```bash
python3 download_tinyllama.py
```

## 3. Verify the model files
```bash
ls models/tinyllama
```

## 4. Build and run the backend server
```bash
cargo run --release
```

## 5. Test the chat endpoint
```bash
curl -X POST http://localhost:8000/chat \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Hello TinyLlama!", "max_tokens": 32}'
```
