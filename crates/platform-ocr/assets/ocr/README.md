# Bundled OCR models

These are the PaddleOCR PP-OCRv6 tiny detection and recognition models used by
the Windows OCR backend.

- Source: `PaddlePaddle/PP-OCRv6_tiny_det_onnx` and
  `PaddlePaddle/PP-OCRv6_tiny_rec_onnx` on Hugging Face, exported to ONNX by
  the PaddleOCR project.
- License: Apache-2.0. See `LICENSE-APACHE`.
- `ppocrv6_dict.txt` is the recognition dictionary extracted from the model's
  `inference.yml`, with the final space class appended (6905 lines; the CTC
  blank class is implicit). The recognition model outputs 6906 classes.
- Large screenshots are detected in overlapping 1280 px tiles so 4K UI text is
  not shrunk to a few pixels; the resulting boxes are merged before
  recognition.

The models are embedded in the Windows x64 and ARM64 binaries with
`include_bytes!`.
