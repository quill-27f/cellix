#!/usr/bin/env python3
"""Writes the Homebrew formula for a release, from the checksums of its archives.

    formula.py <version> <folder of .tar.gz files> <out.rb>
"""
import hashlib, os, sys

version, folder, out = sys.argv[1:4]
URL = "https://github.com/quill-27f/cellix/releases/download/v{}/cellix-{}.tar.gz"


def entry(target):
    path = os.path.join(folder, f"cellix-{target}.tar.gz")
    with open(path, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()
    return f'url "{URL.format(version, target)}"\n      sha256 "{digest}"'


formula = f'''class Cellix < Formula
  desc "AI companion IDE for your terminal"
  homepage "https://github.com/quill-27f/cellix"
  version "{version}"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      {entry("aarch64-apple-darwin")}
    end
    on_intel do
      {entry("x86_64-apple-darwin")}
    end
  end

  on_linux do
    on_arm do
      {entry("aarch64-unknown-linux-musl")}
    end
    on_intel do
      {entry("x86_64-unknown-linux-musl")}
    end
  end

  def install
    bin.install "cellix"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/cellix --version")
  end
end
'''
with open(out, "w") as f:
    f.write(formula)
