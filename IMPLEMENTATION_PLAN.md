# Luty implementation plan

Luty is a dark-only desktop image finishing tool. The first release optimizes for a short path: choose a LUT library once, drop an image, audition looks, adjust strength, and export.

## Product flow

1. On first launch, ask for the folder containing `.cube` LUTs. Remember it locally and allow it to be changed from Settings.
2. Accept an image through the native file picker or Tauri file-drop events. Decode in Rust, never in the webview for final processing.
3. Scan the selected folder recursively, parse supported LUT metadata, and show a searchable gallery. Invalid files remain isolated and never block the rest of the library.
4. Generate preview renders at a bounded resolution in Rust. Cache by image fingerprint, LUT fingerprint, intensity, and preview size.
5. Apply the selected LUT at full source resolution for export. Preserve the source aspect ratio and offer PNG, JPEG, and WebP output initially.

## Architecture

- **SvelteKit UI:** orchestration, native event handling, rune-backed contextual state, accessible controls, and image presentation.
- **Tauri commands:** settings, filesystem selection, LUT discovery, preview rendering, and export.
- **Rust image pipeline:** decode, normalize pixels, trilinear LUT interpolation, intensity blend, encode.
- **Persistence:** a small JSON settings file in Tauri's app config directory. Image data and LUT tables stay out of webview state.

## Delivery slices

- [x] Dark desktop workspace and responsive empty/working states
- [x] Typed frontend bridge with browser-preview fallbacks
- [x] Native LUT directory discovery and `.cube` parsing
- [x] Native image processing and export command scaffold
- [ ] Debounced preview cache and cancellation for rapid LUT auditioning
- [ ] HEIC/AVIF decoder feature detection and optional platform codecs
- [ ] Embedded starter LUT pack with licensing metadata
- [ ] Color-management pass (embedded ICC profiles, linear-light option)
- [ ] Visual regression fixtures and representative benchmark suite

## Performance guardrails

- Keep decoded pixels and parsed LUTs on the Rust side.
- Bound preview dimensions and avoid base64 for full-resolution image transfers.
- Parse each LUT once per fingerprint and reuse its contiguous RGB table.
- Parallelize pixel transforms only when image size makes the scheduling cost worthwhile.
- Write exports atomically and return paths plus dimensions, not image blobs.
