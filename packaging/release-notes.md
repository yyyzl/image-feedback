# Image Feedback v0.1.1

Native, local-first image feedback editor for Windows and macOS.

## Changes in v0.1.1

- Dedicated monochrome macOS menu-bar icon, using an AppKit template image for automatic light/dark and selection tinting.
- Dock, Applications and Windows icons retain the original full-color artwork.
- Verify the actual menu-bar image template flag on both Apple Silicon and Intel.
- Window regressions wait for image loading and analysis readiness with a bounded timeout, rather than failing solely because a CI machine needs more than two seconds to start.
- Add an atomic sequence to temporary/export names, preventing same-process collisions when the Windows clock returns identical timestamps; verified with a fixed-clock multithreaded regression.

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
