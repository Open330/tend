import AppKit

/// The app's windows as the person sees them.
@MainActor
enum TendAppWindows {
    /// Whether any window of the app is on screen: the workbench, Settings,
    /// a detached module, the menu bar's panel. Not the status item itself,
    /// which is always there.
    static func anyOnScreen() -> Bool {
        NSApp.windows.contains { window in
            window.isVisible && !window.isMiniaturized && window.occlusionState.contains(.visible)
                && !String(describing: type(of: window)).contains("StatusBar")
        }
    }
}
