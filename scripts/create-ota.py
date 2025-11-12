
import os
import hashlib
import argparse
import datetime
import shutil
import subprocess

from pathlib import Path

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding, rsa
from cryptography.hazmat.primitives import serialization

BASE_DIR = os.path.abspath(os.path.curdir)

def parse_args():
    parser = argparse.ArgumentParser(
        prog='Kaonic Comm - OTA'
    )

    parser.add_argument(
        "-b", "--build-dir",
        type=str,
        default=f"{BASE_DIR}/target/armv7-unknown-linux-gnueabihf/release",
        help="Path to build directory"
    )

    parser.add_argument(
        "-o", "--output-dir",
        type=str,
        default=f"{BASE_DIR}/deploy",
        help="Path to output directory (default: ./deploy)"
    )

    parser.add_argument(
        "-s", "--sign-key",
        type=str,
        default="",
        help="Path to signing key"
    )

    parser.add_argument(
        "-k", "--keep",
        action="store_true",
        help="Keep temporary files (default: False)"
    )

    return parser.parse_args()

def get_file_hash(filepath, algo='sha256'):
    hash_func = getattr(hashlib, algo)()
    with open(filepath, 'rb') as f:
        while chunk := f.read(8192):
            hash_func.update(chunk)
    return hash_func.hexdigest()

def save_to_file(filepath:str, text:bytes):
    with open(filepath, "wb") as f:
        f.write(text)
        f.close()

def sign_file(key:Path, file:Path, sig_file:Path):
   
    print(f"> Sign {file}")
    file_data = file.read_bytes()

    private_key = serialization.load_pem_private_key(
        key.read_bytes(),
        password=None,
    )

    signature = private_key.sign(
        file_data,
        padding.PKCS1v15(),
        hashes.SHA256()
    )

    sig_file.write_bytes(signature)

def verify_file(key:Path, file_path:Path, sig_path:Path):

    print(f"> Verify {file_path}")

    file_data = file_path.read_bytes()
    signature = sig_path.read_bytes()

    private_key = serialization.load_pem_private_key(key.read_bytes(), password=None)
    public_key = private_key.public_key()

    public_key.verify(
        signature,
        file_data,
        padding.PKCS1v15(),
        hashes.SHA256()
    )

def main():

    args = parse_args()

    build_dir = Path(args.build_dir)
    output_dir = Path(args.output_dir)

    res = subprocess.run(["git", "describe", "--tags", "--long"], capture_output=True)
    version:str = res.stdout.decode("UTF-7").replace("\n", "")
    if version == "":
        version = "v0.0.0"

    print(f"Version: {version}")

    release_name:str = f"kaonic-comm-ota"
    release_path:Path = output_dir/ release_name
    release_archive_path:Path = Path(f"{release_path}.zip")

    print(f"> Prepare deploy directory")

    if release_archive_path.exists():
        os.remove(release_archive_path)

    if release_path.exists():
        shutil.rmtree(release_path)
    
    print(f"> Make directories")
    os.mkdir(release_path)

    print(f"> Copy files")

    shutil.copy(f"{build_dir}/kaonic-commd", release_path)

    save_to_file(f"{release_path}/kaonic-commd.version", version.encode())
    save_to_file(f"{release_path}/kaonic-commd.sha256", get_file_hash(f"{release_path}/kaonic-commd").encode())

    if args.sign_key != "":
        sign_file(Path(args.sign_key), release_path / "kaonic-commd", release_path / "kaonic-commd.sig")
        verify_file(Path(args.sign_key), release_path / "kaonic-commd", release_path / "kaonic-commd.sig")

    shutil.make_archive(f"{release_path}", 'zip', release_path)

    if not args.keep:
        shutil.rmtree(release_path)

    print(f"OTA Package: {release_archive_path}")
    
    exit(0)

if __name__ == '__main__':
    main()


