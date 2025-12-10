# Setup Instructions

Follow these steps to set up and run the TinyLlama Candle Chat Backend.

## 1. Install the Hugging Face Hub Python client
```bash
pip install huggingface_hub
```

## 2. python3 download_tinyllama.py
```bash
python3 download_qwen_instruct.py
```

## 3. Verify the model files
```bash
ls models/qwen2_0_5b_instruct
```

## 4. Build and run the backend server
```bash
cargo run --release
```

## 5. Test the chat endpoint
```bash
curl -N "http://localhost:8001/chat/stream?prompt=Hello%20Qwen%2C%20how%20are%20you&max_tokens=64"
```
