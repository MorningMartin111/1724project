from huggingface_hub import snapshot_download

# Download TinyLlama into ./models/tinyllama
snapshot_download(
    repo_id="TinyLlama/TinyLlama-1.1B-Chat-v1.0",
    local_dir="models/tinyllama",
    local_dir_use_symlinks=False,
)

print("✅ Download complete! Files are in models/tinyllama")
