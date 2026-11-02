// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "IrisShareCore",
    platforms: [.macOS(.v13), .iOS(.v16)],
    targets: [
        .target(name: "IrisShareCore", path: "Sources"),
        .testTarget(name: "IrisShareCoreTests", dependencies: ["IrisShareCore"], path: "Tests"),
    ],
    swiftLanguageVersions: [.v5]
)
