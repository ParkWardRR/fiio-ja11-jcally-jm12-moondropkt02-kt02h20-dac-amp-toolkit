// swift-tools-version: 6.0
//
// `ktmac` — the macOS-native half of ktflash (docs/MACOS-NATIVE.md).
//
// Deliberately DEPENDENCY-FREE: no swift-argument-parser, nothing from the network. This is a
// tool that helps erase firmware, so `swift build` offline with zero third-party code is worth
// more than a nicer CLI parser.
//
// NO TEST TARGET, on purpose. Neither XCTest nor swift-testing is available with only the
// Command Line Tools installed (`xcode-select -p` → /Library/Developer/CommandLineTools), and
// requiring a full Xcode.app to run the checks would mean they mostly don't get run. The
// assertions live in `KTMacKit/SelfTest.swift` and run as `ktmac selftest` with no framework
// at all — see the rationale there. If you add Xcode later, porting them to swift-testing is
// mechanical.

import PackageDescription

let package = Package(
    name: "ktmac",
    platforms: [.macOS(.v12)],
    products: [
        .library(name: "KTMacKit", targets: ["KTMacKit"]),
        .executable(name: "ktmac", targets: ["ktmac"]),
    ],
    targets: [
        .target(name: "KTMacKit"),
        .executableTarget(name: "ktmac", dependencies: ["KTMacKit"]),
    ]
)
