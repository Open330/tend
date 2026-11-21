// Read every PNG in a directory with Vision and print `file<TAB>line` for
// each recognised line (the demo video's privacy pass, scripts/demo-video).
import Foundation
import Vision

let dir = URL(fileURLWithPath: CommandLine.arguments[1])
let files = try FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)
    .filter { $0.pathExtension == "png" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
for file in files {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["en-US", "ko-KR"]
    try VNImageRequestHandler(url: file).perform([request])
    for observation in request.results ?? [] {
        if let line = observation.topCandidates(1).first?.string {
            print("\(file.lastPathComponent)\t\(line)")
        }
    }
}
