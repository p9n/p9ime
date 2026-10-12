#!/usr/bin/env bash
# Regenerates assets/icon/p9ime.ico from p9ime.svg (32..256px) and
# p9ime-small.svg (16/20/24px) using ImageMagick.
set -euo pipefail

cd "$(dirname "$0")"

magick -background none \
  \( p9ime-small.svg -resize 16x16 \) \
  \( p9ime-small.svg -resize 20x20 \) \
  \( p9ime-small.svg -resize 24x24 \) \
  \( p9ime.svg -resize 32x32 \) \
  \( p9ime.svg -resize 48x48 \) \
  \( p9ime.svg -resize 64x64 \) \
  \( p9ime.svg -resize 128x128 \) \
  \( p9ime.svg -resize 256x256 \) \
  p9ime.ico
