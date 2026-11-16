// Pairing: the phone asks the dashboard, an operator approves the code it
// shows. The key the dashboard then issues never leaves this phone.
import SwiftUI
import UIKit

struct PairingView: View {
    @Environment(AppModel.self) private var model
    @State private var address = ProcessInfo.processInfo.environment["TEND_SERVER"]
        ?? UserDefaults.standard.string(forKey: "lastServer") ?? ""
    @State private var name = ""
    @State private var scope: DeviceScope = .operate

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    HStack(spacing: 12) {
                        TendMark(waiting: true, error: false, working: true, done: true)
                            .frame(width: 44, height: 44)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Tend").font(.largeTitle.bold())
                            Text("Decide what your agents ask you, from here.")
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                    }
                    switch model.phase {
                    case .waitingForApproval(let origin, let userCode, let expires):
                        waiting(origin: origin, userCode: userCode, expires: expires)
                    default:
                        form
                    }
                }
                .padding(20)
                .frame(maxWidth: 560, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .background(TendSurface.panel)
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Pair with your dashboard")
                .font(.title3.bold())
            Text("Enter the address of the tend dashboard you use in the browser. This device gets its own key for every host that dashboard covers. It never types into panes.")
                .font(TendType.body)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 6) {
                Text("Dashboard address").font(.footnote.weight(.semibold))
                TextField("https://tend.example.com", text: $address)
                    .textContentType(.URL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("address")
                    .submitLabel(.next)
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("Name for this device").font(.footnote.weight(.semibold))
                TextField(UIDevice.current.name, text: $name)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("deviceName")
                    .submitLabel(.go)
                    .onSubmit(start)
            }
            VStack(alignment: .leading, spacing: 8) {
                Text("What this device may do").font(.footnote.weight(.semibold))
                ScopeChoice(scope: $scope, value: .operate,
                            title: String(localized: "Decide, see hosts and start work"),
                            detail: String(localized: "Answer what needs you; see every host and agent and their recent output; start new work, which runs agents on your hosts (Face ID first)."))
                ScopeChoice(scope: $scope, value: .decide,
                            title: String(localized: "Decide only"),
                            detail: String(localized: "Read and answer what needs you. An answer can still tell an agent to go ahead."))
                Text("The operator who approves this device on the dashboard chooses too, and may allow deciding only.")
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }
            if let problem = model.pairingProblem {
                Label(problem, systemImage: "exclamationmark.circle")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("pairingProblem")
            }
            Button(action: start) {
                Text("Pair").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(address.trimmingCharacters(in: .whitespaces).isEmpty)
            .accessibilityIdentifier("pairButton")
        }
    }

    private func waiting(origin: ServerOrigin, userCode: String, expires: Date) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Approve this phone")
                .font(.title3.bold())
            Text("On a computer where you are signed in to \(origin.display), open **access ▸ Devices** and enter this code:")
                .font(TendType.body)
                .fixedSize(horizontal: false, vertical: true)
            Text(verbatim: userCode)
                .font(.system(size: 40, weight: .bold, design: .monospaced))
                .kerning(2)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
                .background(TendColor.waiting.opacity(0.14), in: RoundedRectangle(cornerRadius: 12))
                .accessibilityIdentifier("userCode")
            HStack(spacing: 8) {
                ProgressView()
                Text("Waiting for approval · the code expires \(expires, style: .relative)")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Text("Check that the dashboard shows this phone's name before you approve it.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            Button("Cancel", role: .cancel) { model.cancelPairing() }
                .accessibilityIdentifier("cancelPairing")
        }
    }

    private func start() {
        UserDefaults.standard.set(address, forKey: "lastServer")
        model.startPairing(address: address, name: name, scope: scope)
    }
}

/// One of the two scopes, as a selectable card.
private struct ScopeChoice: View {
    @Binding var scope: DeviceScope
    let value: DeviceScope
    let title: String
    let detail: String

    var body: some View {
        let selected = scope == value
        Button {
            scope = value
        } label: {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(selected ? Color.accentColor : Color.secondary)
                    .font(.title3)
                VStack(alignment: .leading, spacing: 3) {
                    Text(title).font(TendType.body.weight(.semibold)).foregroundStyle(.primary)
                    Text(detail).font(TendType.meta).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .tendCard(stroke: selected ? Color.accentColor.opacity(0.6) : TendSurface.border, lineWidth: selected ? 1.5 : 1)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("scope-\(value.rawValue)")
    }
}
