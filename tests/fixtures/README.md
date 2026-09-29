# Decoder fixtures

Generated locally with Pillow 12.3.0; no external artwork.

- `rgba.png` / `rgba.webp`: 2×2 static RGBA, red/green/transparent blue/white.
- `rgb.jpg`: the RGB conversion of the same fixture.
- `rgba-lossy.webp`: lossy VP8 with alpha and an extended canvas header.
- `gray16.png`: 1×1 16-bit grayscale white, normalized to RGBA8 on import.
- `animated.png` / `animated.webp`: two distinct frames; import must reject them.
- `huge-header.png`: RGBA PNG with a recomputed IHDR CRC and width 100000;
  import must reject its header dimensions before expanding the pixel payload.
