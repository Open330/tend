# Push setup (iOS)

How to turn on notifications for Tend for iOS on the host that serves the
dashboard. Once the Apple key exists, this takes about five minutes. What tendd
does with the key, and why, is in [DASHBOARD.md](DASHBOARD.md#push-notifications-ios).

You need the Apple developer team that publishes the app, and a shell on the
host that runs the dashboard's tendd.

## 1. Apple: enable Push on the App ID (once)

[developer.apple.com](https://developer.apple.com/account) ▸ Certificates,
IDs & Profiles ▸ **Identifiers** ▸ `dev.tend.ios` ▸ tick **Push
Notifications** ▸ Save.

Release builds carry `aps-environment = production`. After this change,
regenerate the App Store provisioning profile (**Profiles** ▸ the app's App
Store profile ▸ Edit ▸ Save ▸ Download) and install it. Otherwise the next
archive fails to export.

## 2. Apple: create the APNs key (once)

**Keys** ▸ **+**:

1. Name: anything, for example `Tend push`.
2. Tick **Apple Push Notifications service (APNs)** ▸ Configure. For the
   environment, choose **Production**: TestFlight and App Store builds use
   it. tendd holds one key, so a Debug build from Xcode (`development`)
   needs a key that also covers Sandbox. Choose **Sandbox & Production**
   only if the portal offers it for one key.
3. For the key restriction, choose **Topic Specific** and select
   `dev.tend.ios`, the same value as `topic` in section 4 below. Do not use
   **Team Scoped**: that key can push to every app on the team, so if the
   host leaked it, it would expose more than Tend.
4. Continue ▸ Register ▸ **Download**. The file is `AuthKey_<KEYID>.p8`.
   Apple lets you download it **once**, so put a copy in your password
   vault now.

Note two values:

| Value | Where Apple shows it | Example |
|---|---|---|
| Key ID | The key's page; also the file name | `ABC123DEFG` |
| Team ID | Membership details (top right ▸ your team) | `728FW73BS8` |

## 3. The host: install the key

On the host that serves the dashboard, signed in as the user tendd runs as:

```sh
mkdir -p ~/.config/tend/apns && chmod 700 ~/.config/tend/apns
install -m 600 AuthKey_ABC123DEFG.p8 ~/.config/tend/apns/
```

tendd refuses the key if it is not a regular file (a symlink, for
example), is not owned by tendd's user, has any group or other permission
bit (`0600` is right), is larger than 4 KiB, or is not a PKCS#8
P-256 PEM. The `.p8` Apple gives you is that.

## 4. The host: `[dashboard.push]`

Add this to the config file tendd loaded (`tend config path` prints it;
usually `~/.config/tend/config.toml`), or in Tend.app ▸ Settings ▸
Advanced. `tend config set --from` replaces the **whole** file, so only
give it the complete edited document (`tend config show` prints the
current one), never this section alone.

```toml
[dashboard.push]
enabled = true
key_path = "~/.config/tend/apns/AuthKey_ABC123DEFG.p8"  # step 3
key_id = "ABC123DEFG"           # step 2, Key ID
team_id = "728FW73BS8"          # step 2, Team ID
topic = "dev.tend.ios"          # the app's bundle id (the default)
environments = ["production"]   # TestFlight and App Store builds (the default)
allow_content = false           # true: notifications may show the question's text
```

- `environments`: TestFlight and App Store builds register as
  `production`. Add `"development"` only for Debug builds run from Xcode.
- `allow_content`: with `false`, a notification says only which host needs
  you and how many. With `true`, and **Show what was asked** on in the app,
  it also shows the first line of the item and who asked. It then offers
  **Looks good** / **Send back** / **Answer**, which work only on an
  unlocked phone.

Then restart tendd:

```sh
tend daemon restart
```

## 5. Check

- **access ▸ Devices** on the dashboard says "Notifications: on
  (production)". If `[dashboard.push]` is there but cannot be used, it says
  "Notifications are off" and why: for example a key with the wrong mode, a
  malformed id or an unknown field. The same reason is logged once when
  tendd starts, and `GET /api/devices` returns it as `push.reason`. With
  `enabled = false`, or no section, it says nothing about notifications.
- On the phone: the app's Settings ▸ **Notifications** ▸ **Notify me when
  something needs me** (Your turn offers it once, too). iOS asks for
  permission. The next new question or result raises a
  notification within about 30 seconds.

A bad `[dashboard.push]` never stops tendd: push stays off and everything
else keeps working.

## Turning it off

Set `enabled = false` (or remove the section) and restart tendd. To revoke
the key, delete it under **Keys** on developer.apple.com. To rotate it,
create a new key, change `key_path` and `key_id`, restart tendd, and then
revoke the old key.
