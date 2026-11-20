import AppKit
import SwiftUI

/// What's New in Tend X.Y.Z: opened once after an upgrade instead of the
/// full Welcome guide (see `OnboardingPreferences.launchPresentation`), and
/// from Help › What's New in Tend.
struct WhatsNewView: View {
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme
    @State private var hostWindow: NSWindow?

    private let version = OnboardingPreferences.currentVersion

    private var releases: [TendWhatsNew.Release] {
        TendWhatsNew.releasesToShow(for: version)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .center, spacing: TendSpace.m) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 48, height: 48)
                VStack(alignment: .leading, spacing: TendSpace.xs) {
                    Text("What's New in Tend \(releases.first?.version ?? version)")
                        .font(TendType.page)
                    Text("Highlights of this release.")
                        .font(TendType.detail)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(TendSpace.xl)

            TendTheme.border(colorScheme).frame(height: 1)

            ScrollView {
                VStack(alignment: .leading, spacing: TendSpace.l) {
                    ForEach(releases.flatMap(\.highlights)) { highlight in
                        HStack(alignment: .top, spacing: TendSpace.m) {
                            Image(systemName: highlight.systemImage)
                                .font(TendType.icon)
                                .foregroundStyle(Color.accentColor)
                                .frame(width: 26)
                            VStack(alignment: .leading, spacing: TendSpace.xxs) {
                                Text(verbatim: highlight.title)
                                    .font(TendType.section)
                                Text(verbatim: highlight.detail)
                                    .font(TendType.detail)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    if releases.isEmpty {
                        Text("No highlights were recorded for this version.")
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(TendSpace.xl)
            }

            TendTheme.border(colorScheme).frame(height: 1)

            HStack(spacing: TendSpace.s) {
                Button("Open Welcome Guide") {
                    openWindow(id: OnboardingPreferences.windowID)
                    close()
                }
                .buttonStyle(.tendSecondary)
                Spacer()
                Button("Continue") { close() }
                    .buttonStyle(.tendPrimary)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(TendSpace.m)
        }
        .frame(width: 520, height: 540)
        .background(TendTheme.editor(colorScheme))
        .background(OnboardingWindowTracker(identifier: OnboardingPreferences.whatsNewWindowIdentifier) { window in
            if let window { hostWindow = window }
        })
        .onExitCommand(perform: close)
    }

    private func close() {
        (hostWindow ?? OnboardingPreferences.existingWindow(identifier: OnboardingPreferences.whatsNewWindowIdentifier))?
            .close()
    }
}
