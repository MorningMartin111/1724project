from huggingface_hub import snapshot_download


def main():
    repo_id = "Qwen/Qwen2.5-0.5B-Instruct"

    print(f"📥 Downloading {repo_id} ...")
    snapshot_download(
        repo_id=repo_id,
        local_dir="models/qwen2_0_5b_instruct",
        local_dir_use_symlinks=False,
    )
    print("✅ Download complete!")


if __name__ == "__main__":
    main()
