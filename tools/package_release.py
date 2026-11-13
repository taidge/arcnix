"""Package a native release binary using Cargo's actual target directory."""
import argparse
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "aarch64-apple-darwin": "macos-arm64",
    "x86_64-pc-windows-msvc": "windows-amd64",
    "aarch64-unknown-linux-gnu": "linux-arm64",
    "x86_64-unknown-linux-gnu": "linux-amd64",
}


def macos_bundle(binary, directory, version):
    bundle = directory / "Arcnix.app"
    contents = bundle / "Contents"
    executable = contents / "MacOS" / "arcnix"
    resources = contents / "Resources"
    executable.parent.mkdir(parents=True)
    resources.mkdir()
    shutil.copy2(binary, executable)
    executable.chmod(0o755)
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump({
            "CFBundleIdentifier": "com.taidge.arcnix",
            "CFBundleName": "Arcnix",
            "CFBundleDisplayName": "Arcnix",
            "CFBundleExecutable": "arcnix",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version.split("-")[0],
            "CFBundleVersion": version.split("-")[0],
            "CFBundleIconFile": "arcnix.icns",
            "LSMinimumSystemVersion": "13.0",
            "NSHighResolutionCapable": True,
            "NSPrincipalClass": "NSApplication",
        }, stream)
    iconset = directory / "arcnix.iconset"
    iconset.mkdir()
    source = ROOT / "crates/arcnix/assets/arcnix.png"
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            pixels = size * scale
            if pixels > 512:
                continue
            suffix = "@2x" if scale == 2 else ""
            image = iconset / f"icon_{size}x{size}{suffix}.png"
            subprocess.run(
                ["sips", "-z", str(pixels), str(pixels), str(source), "--out", str(image)],
                check=True, stdout=subprocess.DEVNULL,
            )
    subprocess.run(
        ["iconutil", "-c", "icns", str(iconset), "-o", str(resources / "arcnix.icns")],
        check=True,
    )
    # The iconset is a build intermediate, outside the application bundle.
    subprocess.run(["codesign", "--force", "--sign", "-", str(bundle)], check=True)
    return bundle


def archive_permissions(info):
    info.mode = 0o755 if info.isdir() or info.name.endswith("/arcnix") else 0o644
    return info


def package(binary, target, tag, output, version):
    platform = TARGETS[target]
    stem = f"arcnix-{tag}-{platform}"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="arcnix-package-") as temporary:
        directory = Path(temporary)
        staging = directory / stem
        staging.mkdir()
        if platform == "macos-arm64":
            bundle = macos_bundle(binary, directory, version)
            shutil.move(str(bundle), staging / bundle.name)
        else:
            executable = staging / binary.name
            shutil.copy2(binary, executable)
            executable.chmod(0o755)
        for name in ("LICENSE", "README.md", "README.zh-CN.md"):
            shutil.copy2(ROOT / name, staging / name)
        vendor = ROOT / "crates/arcnix/assets/vendor"
        for name in ("xterm", "addon-fit"):
            licenses = staging / "LICENSES" / name
            licenses.mkdir(parents=True)
            shutil.copy2(vendor / name / "LICENSE", licenses / "LICENSE")
        shutil.copy2(vendor / "README.md", staging / "LICENSES/README.md")
        if platform == "windows-amd64":
            archive = output / f"{stem}.zip"
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as stream:
                for path in sorted(staging.rglob("*")):
                    if path.is_file():
                        stream.write(path, path.relative_to(directory).as_posix())
        else:
            archive = output / f"{stem}.tar.gz"
            with tarfile.open(archive, "w:gz") as stream:
                stream.add(staging, arcname=stem, filter=archive_permissions)
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT, text=True,
    ))
    version = next(item["version"] for item in metadata["packages"] if item["name"] == "arcnix")
    if args.tag != f"v{version}":
        parser.error(f"Tag {args.tag} does not match arcnix version {version}")
    name = "arcnix.exe" if args.target == "x86_64-pc-windows-msvc" else "arcnix"
    binary = Path(metadata["target_directory"]) / args.target / "release" / name
    if not binary.is_file():
        parser.error(f"Release binary not found: {binary}")
    print(package(binary, args.target, args.tag, args.output.resolve(), version))


if __name__ == "__main__":
    main()
