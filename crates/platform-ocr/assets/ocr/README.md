# Bundled OCR models

These files are the PaddleOCR PP-OCRv3 Chinese detection and recognition
models used by the Windows OCR backend.

- Source: PaddleOCR / RapidOCR redistributions (`ch_PP-OCRv3_det_infer.onnx`,
  `ch_PP-OCRv3_rec_infer.onnx`).
- License: Apache-2.0. See `LICENSE-APACHE`.
- `ppocr_keys_v1.txt` is the recognition dictionary extracted from the model
  metadata, with the final space class appended.
- `ch_PP-OCRv3_rec_static.onnx` is the upstream recognition model with the
  input shape fixed to `[1, 3, 48, 320]`. `paddleocr_rs_onnx` reads the
  height/width from the ONNX input shape; the upstream model uses symbolic
  dimensions and would otherwise be treated as height 1.

The models are embedded in the Windows x64 and ARM64 binaries with
`include_bytes!`.
