// swift-tools-version: 5.10
// SJTU Canvas Downloader for macOS. `./build.sh` assembles the .app around
// this executable; the package can also be opened in Xcode for editing.
import PackageDescription

let package = Package(
    name: "SJTUCanvasDownloader",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "SJTUCanvasDownloader", targets: ["SJTUCanvasDownloader"]),
    ],
    targets: [
        .executableTarget(
            name: "SJTUCanvasDownloader",
            path: "Sources/SJTUCanvasDownloader"
        ),
    ]
)
