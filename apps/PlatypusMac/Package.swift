// swift-tools-version:5.9
import PackageDescription

// The Rust static lib (libplatypus_ffi.a) is built by cargo into the workspace
// target dir. `just app::build` builds it (--release) and syncs the header before
// `swift build`. Release matters: the full-USA parse/index is CPU-bound and
// debug Rust loads ~6x slower. Linker flags below match what the C smoke test
// needed for a Rust staticlib on macOS (Rust std pulls in CoreFoundation /
// Security / iconv).
let rustLibDir = "../../target/release"

let package = Package(
    name: "PlatypusMac",
    platforms: [.macOS(.v14)],
    targets: [
        // Thin C module exposing platypus.h to Swift.
        .target(name: "CPlatypusFFI"),

        // The SwiftUI app, linking the prebuilt Rust static lib.
        .executableTarget(
            name: "PlatypusMac",
            dependencies: ["CPlatypusFFI"],
            linkerSettings: [
                .unsafeFlags(rustLinkFlags)
            ]
        ),

        // Unit tests over the pure logic (`@testable import PlatypusMac`). Re-links the Rust
        // staticlib because the imported app code calls into the FFI.
        .testTarget(
            name: "PlatypusMacTests",
            dependencies: ["PlatypusMac"],
            linkerSettings: [
                .unsafeFlags(rustLinkFlags)
            ]
        ),
    ]
)

// Flags to link the prebuilt Rust staticlib on macOS (Rust std pulls in CoreFoundation /
// Security / iconv). Shared by the app + its test target.
var rustLinkFlags: [String] {
    [
        "-L\(rustLibDir)",
        "-lplatypus_ffi",
        "-framework", "CoreFoundation",
        "-framework", "Security",
        "-liconv",
    ]
}
