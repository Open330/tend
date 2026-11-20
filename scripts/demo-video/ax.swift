// A small accessibility helper for the demo-video driver (CI runner only).
//
//   ax trusted                       exit 0 when this process may use AX
//   ax find PID TEXT [ROLE]          "x y w h" (screen points) of the first
//                                    element whose title, description, value,
//                                    label or identifier contains TEXT
//   ax tree PID                      dump the window tree (debugging)
//   ax find-extra PID TEXT [ROLE] the same, in the menu bar extras only
//   ax extras PID                    frames of the app's menu bar extras
//   ax raise PID X Y W H             move/resize the app's first window
import AppKit
import ApplicationServices

func attr(_ e: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    return AXUIElementCopyAttributeValue(e, name as CFString, &value) == .success ? value : nil
}
func string(_ e: AXUIElement, _ name: String) -> String {
    guard let v = attr(e, name) else { return "" }
    if let s = v as? String { return s }
    if let n = v as? NSNumber { return n.stringValue }
    return ""
}
func frame(_ e: AXUIElement) -> CGRect? {
    guard let p = attr(e, kAXPositionAttribute), let s = attr(e, kAXSizeAttribute) else { return nil }
    var point = CGPoint.zero, size = CGSize.zero
    AXValueGetValue(p as! AXValue, .cgPoint, &point)
    AXValueGetValue(s as! AXValue, .cgSize, &size)
    return CGRect(origin: point, size: size)
}
func children(_ e: AXUIElement) -> [AXUIElement] {
    (attr(e, kAXChildrenAttribute) as? [AXUIElement]) ?? []
}
func label(_ e: AXUIElement) -> String {
    [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute, kAXPlaceholderValueAttribute, "AXLabel", kAXIdentifierAttribute, kAXHelpAttribute]
        .map { string(e, $0) }.filter { !$0.isEmpty }.joined(separator: " | ")
}
func windows(_ app: AXUIElement) -> [AXUIElement] {
    (attr(app, kAXWindowsAttribute) as? [AXUIElement]) ?? []
}

let args = CommandLine.arguments
guard args.count >= 2 else { exit(2) }
switch args[1] {
case "trusted":
    exit(AXIsProcessTrusted() ? 0 : 1)
case "find", "find-extra":
    let app = AXUIElementCreateApplication(pid_t(args[2])!)
    let text = args[3].lowercased()
    let role = args.count > 4 ? args[4] : nil
    let extras = (attr(app, "AXExtrasMenuBar")).map { [$0 as! AXUIElement] } ?? []
    var queue = args[1] == "find-extra" ? extras : windows(app) + extras
    var seen = 0
    while !queue.isEmpty, seen < 20000 {
        let e = queue.removeFirst(); seen += 1
        if label(e).lowercased().contains(text),
           role == nil || string(e, kAXRoleAttribute) == role,
           let f = frame(e), f.width > 0, f.height > 0 {
            print("\(Int(f.minX)) \(Int(f.minY)) \(Int(f.width)) \(Int(f.height))")
            exit(0)
        }
        queue.append(contentsOf: children(e))
    }
    exit(1)
case "tree":
    let app = AXUIElementCreateApplication(pid_t(args[2])!)
    func dump(_ e: AXUIElement, _ depth: Int) {
        guard depth < 40 else { return }
        let f = frame(e).map { "\(Int($0.minX)),\(Int($0.minY)) \(Int($0.width))x\(Int($0.height))" } ?? "-"
        print(String(repeating: "  ", count: depth) + string(e, kAXRoleAttribute) + " [" + label(e).prefix(120) + "] " + f)
        children(e).forEach { dump($0, depth + 1) }
    }
    windows(app).forEach { dump($0, 0) }
    if let extras = attr(app, "AXExtrasMenuBar") { dump(extras as! AXUIElement, 0) }
case "extras":
    let app = AXUIElementCreateApplication(pid_t(args[2])!)
    guard let extras = attr(app, "AXExtrasMenuBar") else { exit(1) }
    for item in children(extras as! AXUIElement) {
        if let f = frame(item) { print("\(Int(f.minX)) \(Int(f.minY)) \(Int(f.width)) \(Int(f.height))") }
    }
case "raise":
    let app = AXUIElementCreateApplication(pid_t(args[2])!)
    guard let w = windows(app).first else { exit(1) }
    var p = CGPoint(x: Double(args[3])!, y: Double(args[4])!)
    var s = CGSize(width: Double(args[5])!, height: Double(args[6])!)
    AXUIElementSetAttributeValue(w, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &p)!)
    AXUIElementSetAttributeValue(w, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &s)!)
    AXUIElementPerformAction(w, kAXRaiseAction as CFString)
default:
    exit(2)
}
