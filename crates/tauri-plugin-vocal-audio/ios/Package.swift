// swift-tools-version:5.3
import PackageDescription

let package = Package(
    name: "tauri-plugin-vocal-audio",
    platforms: [
        .iOS(.v13)
    ],
    products: [
        .library(
            name: "tauri-plugin-vocal-audio",
            type: .static,
            targets: ["tauri-plugin-vocal-audio"])
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api")
    ],
    targets: [
        .target(
            name: "tauri-plugin-vocal-audio",
            dependencies: [
                .byName(name: "Tauri")
            ],
            path: "Sources")
    ]
)
