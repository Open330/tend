import Testing
@testable import Tend

// Settings › Accounts with the module off: tendd's two reasons for it
// (swap_api::module_unavailable) are said by the pane's own captions, in
// the person's language; anything else tendd says is shown as it is.
@Suite struct AccountsSettingsTests {
    @Test func theTwoModuleOffReasonsAreTheCaptionsAndOthersShow() {
        let configured = "the account module is not enabled: set `[modules] accounts = true` in config.toml (or configure `[agent.<program>].account` / `[[accounts.<provider>]]`) and restart tendd"
        let compiledOut = "the account module is not in this build: rebuild tend-cli and tendd with the `aas` feature (on by default)"
        #expect(TendAccountsSettingsPane.explained(configured))
        #expect(!TendAccountsSettingsPane.notInBuild(configured))
        #expect(TendAccountsSettingsPane.explained(compiledOut))
        #expect(TendAccountsSettingsPane.notInBuild(compiledOut))
        #expect(!TendAccountsSettingsPane.explained("swap store is locked by another tendd"))
        #expect(!TendAccountsSettingsPane.notInBuild(nil))
    }
}
