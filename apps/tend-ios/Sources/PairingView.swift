// Pairing: the phone asks the dashboard, an operator approves the code it
// shows. The key the dashboard then issues never leaves this phone.
import SwiftUI
import UIKit

struct PairingView: View {
    @Environment(AppModel.self) private var model
    @State private var address = ProcessInfo.processInfo.environment["TEND_SERVER"]
        ?? UserDefaults.standard.string(forKey: "lastServer") ?? ""
    @State private var name = ""
    @State private var starting = false

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
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Pair with your dashboard")
                .font(.title3.bold())
            Text("Enter the address of the tend dashboard you use in the browser. This phone gets its own key, which can only read and answer what needs you, on every host that dashboard covers. It cannot see pane output or type into panes. An answer can still tell an agent to go ahead.")
                .font(.callout)
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
            if let problem = model.pairingProblem {
                Label(problem, systemImage: "exclamationmark.circle")
                    .font(.callout)
                    .foregroundStyle(TendColor.error)
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
                .font(.callout)
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
        model.startPairing(address: address, name: name)
    }
}
