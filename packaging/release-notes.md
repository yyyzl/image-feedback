# Image Feedback — early open-source preview

Native, local-first image feedback editor for Windows and macOS.

## Install

- **Windows 10/11 x64:** run the setup EXE. It installs the application, Start Menu entries and uninstaller. CRT is statically linked; Rust, Qt, Node.js and a separate VC Redistributable are not required.
- **macOS Universal:** open the DMG, drag Image Feedback.app to Applications, eject the image and launch from Applications. Both Apple Silicon and Intel slices are included and checked in separate runtime jobs.

**Trust status:** the Windows installer is unsigned. The macOS app is ad-hoc signed, **not Apple-notarized**; first launch may require Open Anyway in Privacy & Security. This release does not claim Gatekeeper or SmartScreen approval. Check the attached SHA-256 files.

## Features

- Region, point and global comments; optional smart region recommendations.
- Move image regions while preserving the original, stable annotation IDs and undo/redo.
- Compact copy: one original-image path plus concise coordinate-based suggestions.
- Portable project save and feedback package export; optional Base64 image data.
- Native macOS top-left window controls and guarded application Quit; custom Windows title bar.
- Installer refuses a running application rather than forcibly discarding edits. Uninstall preserves user data.

Release jobs build the installers, audit dynamic dependencies and run Rust plus copied/installed-application scene regressions. These are not physical mouse/UIA, real IME, clean-VM or minimum-OS certification tests.

See the repository README for usage, limitations, privacy and dependency licensing. No automatic updates or background image uploads.
