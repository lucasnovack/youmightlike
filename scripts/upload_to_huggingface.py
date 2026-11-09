#!/usr/bin/env python3
"""
upload_to_huggingface.py — Uploads exported Parquet dataset and Dataset Card to Hugging Face Hub.

Usage:
  # With explicit token:
  python scripts/upload_to_huggingface.py --repo-id seu-usuario/youmightlike-dataset --token hf_...

  # Or using environment variable:
  export HF_TOKEN="hf_..."
  python scripts/upload_to_huggingface.py --repo-id seu-usuario/youmightlike-dataset
"""

import argparse
import os
import sys

try:
    from huggingface_hub import HfApi, login
except ImportError:
    print("Error: huggingface_hub is not installed.")
    print("Run: pip install huggingface_hub")
    sys.exit(1)

def load_env_file():
    env_path = os.path.join(os.path.dirname(os.path.dirname(__file__)), ".env")
    if os.path.isfile(env_path):
        with open(env_path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#") and "=" in line:
                    k, v = line.split("=", 1)
                    k = k.strip()
                    v = v.strip().strip('"').strip("'")
                    if k not in os.environ:
                        os.environ[k] = v

load_env_file()

def parse_args():
    token_default = os.environ.get("HUGGING_FACE_TOKEN") or os.environ.get("HF_TOKEN")
    parser = argparse.ArgumentParser(description="Upload Parquet dataset to Hugging Face Hub")
    parser.add_argument(
        "--repo-id",
        default="lnvck/youmightlike-dataset",
        help="Target Hugging Face dataset repo id, formatted as 'username/dataset-name' (default: lnvck/youmightlike-dataset)",
    )
    parser.add_argument(
        "--folder",
        default="data/parquet_export",
        help="Folder containing exported Parquet files and README.md (default: data/parquet_export)",
    )
    parser.add_argument(
        "--token",
        default=token_default,
        help="Hugging Face User Access Token (with Write permission). Falls back to HUGGING_FACE_TOKEN / HF_TOKEN env vars.",
    )
    parser.add_argument(
        "--private",
        action="store_true",
        help="Set dataset to private (default is public)",
    )
    return parser.parse_args()

def main():
    args = parse_args()

    if not os.path.isdir(args.folder):
        print(f"Error: Folder '{args.folder}' does not exist.")
        print("Run scripts/export_parquets_for_hf.py first to generate the Parquet files.")
        sys.exit(1)

    files = [f for f in os.listdir(args.folder) if f.endswith(".parquet") or f == "README.md"]
    if not files:
        print(f"Error: No .parquet or README.md files found in '{args.folder}'.")
        sys.exit(1)

    total_size = sum(os.path.getsize(os.path.join(args.folder, f)) for f in files)
    print("=" * 65)
    print(f" Hugging Face Dataset Uploader: {args.repo_id}")
    print(f" Folder: {args.folder} ({len(files)} files, {total_size / (1024*1024*1024):.2f} GB)")
    print(f" Visibility: {'Private' if args.private else 'Public'}")
    print("=" * 65)

    api = HfApi(token=args.token)

    # 1. Verify Authentication
    try:
        user_info = api.whoami(token=args.token)
        username = user_info.get("name") or user_info.get("username")
        print(f" Authenticated as: {username}")
    except Exception as e:
        print(f"\nAuthentication Error: {e}")
        print("\nPlease provide a valid Hugging Face Access Token with WRITE permission:")
        print("  1. Create a token at https://huggingface.co/settings/tokens")
        print("  2. Pass it via --token hf_... or set export HF_TOKEN='hf_...'")
        sys.exit(1)

    # 2. Create Dataset Repo if it doesn't exist
    print(f" Checking / Creating dataset repository '{args.repo_id}'...")
    try:
        api.create_repo(
            repo_id=args.repo_id,
            repo_type="dataset",
            private=args.private,
            exist_ok=True,
            token=args.token,
        )
        print(f" Repository ready at: https://huggingface.co/datasets/{args.repo_id}")
    except Exception as e:
        print(f"Error creating repository: {e}")
        sys.exit(1)

    # 3. Upload all files
    print(f"\n Uploading {len(files)} files to Hugging Face Hub (this may take a few minutes)...")
    try:
        commit_info = api.upload_folder(
            folder_path=args.folder,
            repo_id=args.repo_id,
            repo_type="dataset",
            commit_message="feat: upload youmightlike transmedia cultural dataset and affinity graph",
            token=args.token,
        )
        print("=" * 65)
        print(f" SUCCESS! Dataset successfully uploaded.")
        print(f" View your dataset online:")
        print(f"   https://huggingface.co/datasets/{args.repo_id}")
        print("=" * 65)
    except Exception as e:
        print(f"\nUpload Error: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
