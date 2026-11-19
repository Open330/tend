// The app's side of notifications: the APNs token, the categories and their
// buttons, a tap, and a button pressed on an unlocked phone. Every decision
// goes through `AppModel.act(on:action:text:)`, which reads the item from the
// host first and decides only when it is exactly what the notification
// showed.
import SwiftUI
import UIKit
import UserNotifications

@MainActor
final class PushDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate, ObservableObject {
    let model = AppModel()

    /// The unit tests' host app does nothing of its own.
    private var hostsUnitTests: Bool {
        ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
    }

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        guard !hostsUnitTests else { return true }
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.setNotificationCategories(Self.categories)
        if model.notificationsWanted {
            Task {
                let settings = await center.notificationSettings()
                if settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional {
                    application.registerForRemoteNotifications()
                }
            }
        }
        #if DEBUG
        // UI tests: a notification payload, handed to the same handler.
        if let json = ProcessInfo.processInfo.environment["TEND_TEST_NOTIFICATION"],
           let keyText = ProcessInfo.processInfo.environment["TEND_TEST_PUSH_KEY"],
           let key = Data(base64Encoded: keyText),
           let info = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
           let payload = PushPayload(userInfo: info, category: PushCategory.open, body: "", subtitle: "") {
            Task {
                // After pairing and the first read.
                while model.pairing == nil || !model.loaded { try? await Task.sleep(nanoseconds: 300_000_000) }
                model.adoptTestBinding(PushBinding(channel: payload.channel, refKey: key, content: false))
                // A warm tap: the test moves away from Home meanwhile.
                if let delay = ProcessInfo.processInfo.environment["TEND_TEST_NOTIFICATION_DELAY"].flatMap(Double.init) {
                    try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                }
                await model.open(payload)
            }
        }
        #endif
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { await model.receivedAPNsToken(deviceToken) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // Nothing to register; Settings says notifications are not set up.
    }

    /// Ask iOS, then Apple, for notifications: only when the person turns
    /// them on, never at first launch.
    func enableNotifications() async -> Bool {
        let center = UNUserNotificationCenter.current()
        let granted = (try? await center.requestAuthorization(options: [.alert, .badge, .sound])) ?? false
        guard granted else { return false }
        model.notificationsWanted = true
        UIApplication.shared.registerForRemoteNotifications()
        await model.syncPush()
        return true
    }

    // MARK: Categories

    static var categories: Set<UNNotificationCategory> {
        // Every button needs an unlocked phone; none opens the app.
        let accept = UNNotificationAction(
            identifier: PushCategory.accept,
            title: String(localized: "Looks good"),
            options: [.authenticationRequired]
        )
        let sendBack = UNTextInputNotificationAction(
            identifier: PushCategory.sendBack,
            title: String(localized: "Send back"),
            options: [.authenticationRequired, .destructive],
            textInputButtonTitle: String(localized: "Send back"),
            textInputPlaceholder: String(localized: "What should change?")
        )
        let answer = UNTextInputNotificationAction(
            identifier: PushCategory.answer,
            title: String(localized: "Answer"),
            options: [.authenticationRequired],
            textInputButtonTitle: String(localized: "Send"),
            textInputPlaceholder: String(localized: "Your answer")
        )
        return [
            UNNotificationCategory(identifier: PushCategory.open, actions: [], intentIdentifiers: []),
            UNNotificationCategory(identifier: PushCategory.waiting, actions: [], intentIdentifiers: []),
            UNNotificationCategory(identifier: PushCategory.review, actions: [accept, sendBack], intentIdentifiers: []),
            UNNotificationCategory(identifier: PushCategory.question, actions: [answer], intentIdentifiers: []),
        ]
    }

    // MARK: UNUserNotificationCenterDelegate

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        await model.refresh()
        return [.banner, .list, .badge, .sound]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let content = response.notification.request.content
        guard let payload = PushPayload(userInfo: content.userInfo, category: content.categoryIdentifier, body: content.body, subtitle: content.subtitle) else { return }
        let text = (response as? UNTextInputNotificationResponse)?.userText
        await handle(payload, action: response.actionIdentifier, text: text)
    }

    private func handle(_ payload: PushPayload, action: String, text: String?) async {
        switch action {
        case UNNotificationDefaultActionIdentifier:
            await model.open(payload)
        case UNNotificationDismissActionIdentifier:
            return
        default:
            switch await model.act(on: payload, action: action, text: text) {
            case .done(let done):
                await Self.say(done, opening: nil)
            case .refused:
                // Nothing was decided: offer the item itself.
                await Self.say(String(localized: "Open Tend to decide this."), opening: payload)
            case .unknown:
                // Sent, but the answer was lost: it may have been taken.
                await Self.say(String(localized: "Tend could not confirm this was sent. Open Tend to check."), opening: payload)
            }
        }
    }

    /// A local notification: what a button did, or that it did nothing.
    private static func say(_ text: String, opening payload: PushPayload?) async {
        let content = UNMutableNotificationContent()
        content.title = "Tend"
        content.body = text
        content.categoryIdentifier = PushCategory.open
        if let payload {
            content.userInfo = ["tend": ["v": 1, "channel": payload.channel, "host": payload.host, "ref": payload.ref]]
        }
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        try? await UNUserNotificationCenter.current().add(request)
    }
}

/// Settings ▸ Notifications: what the host offers, the person's switch, and
/// the opt-in to see what was asked on the lock screen.
struct NotificationSettings: View {
    @Environment(AppModel.self) private var model
    @EnvironmentObject private var delegate: PushDelegate
    @State private var wanted = false
    @State private var content = false
    @State private var denied = false

    var body: some View {
        Section {
            if let support = model.pushSupport, support.enabled {
                Toggle(String(localized: "Notify me when something needs me"), isOn: $wanted)
                    .accessibilityIdentifier("notificationsToggle")
                    .onChange(of: wanted) { _, on in
                        Task {
                            if on {
                                denied = !(await delegate.enableNotifications())
                                if denied { wanted = false }
                            } else {
                                await model.turnNotificationsOff()
                            }
                        }
                    }
                if wanted, support.contentAllowed {
                    Toggle(String(localized: "Show what was asked"), isOn: $content)
                        .onChange(of: content) { _, on in
                            model.notificationContentWanted = on
                            Task { await model.registerPush() }
                        }
                }
                if denied {
                    Label(String(localized: "Allow notifications for Tend in the Settings app first."), systemImage: "exclamationmark.triangle")
                        .font(TendType.meta)
                        .foregroundStyle(TendPalette.orange)
                }
                if let problem = model.pushProblem, wanted {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(TendType.meta)
                        .foregroundStyle(TendInk.secondary)
                }
            } else {
                Text(model.pushSupport == nil
                     ? String(localized: "This dashboard's tend cannot send notifications yet. It needs a newer tend.")
                     : String(localized: "This dashboard does not send notifications. Whoever runs it can turn them on with [dashboard.push]."))
                    .font(TendType.meta)
                    .foregroundStyle(TendInk.secondary)
            }
        } header: {
            Text("Notifications")
        } footer: {
            if model.pushSupport?.enabled == true {
                Text(content && model.pushSupport?.contentAllowed == true
                     ? String(localized: "A notification shows the question or result and who asked. The text passes through Apple and shows on the lock screen as far as iOS's Show Previews allows; turning this off stops later ones only. Looks good, Send back and Answer work from the notification once the phone is unlocked.")
                     : String(localized: "A notification says only which host needs you and how many. Tap it to open the item."))
            }
        }
        .onAppear {
            wanted = model.notificationsWanted
            content = model.notificationContentWanted
        }
    }
}

/// Your turn, once: offer notifications when the host sends them.
struct NotificationOffer: View {
    @Environment(AppModel.self) private var model
    @EnvironmentObject private var delegate: PushDelegate
    @AppStorage("notificationOfferDismissed") private var dismissed = false

    var body: some View {
        if model.pushSupport?.enabled == true, !model.notificationsWanted, !dismissed {
            VStack(alignment: .leading, spacing: 8) {
                Label(String(localized: "Get notified when something needs you"), systemImage: "bell.badge")
                    .font(TendType.body.weight(.semibold))
                Text("A notification says only which host needs you and how many; tap it to open the item.")
                    .font(TendType.meta)
                    .foregroundStyle(TendInk.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 10) {
                    Button(String(localized: "Turn on")) {
                        Task {
                            _ = await delegate.enableNotifications()
                            dismissed = true
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("notificationsOn")
                    Button(String(localized: "Not now")) { dismissed = true }
                        .buttonStyle(.borderless)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .tendCard()
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("notificationOffer")
        }
    }
}
