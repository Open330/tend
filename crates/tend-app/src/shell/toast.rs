//! Windows toasts through the `WinRT` projection (safe calls), under the
//! app's `AppUserModelID`. Each toast is tagged with its item's
//! [`identity::toast_tag`], so it can be withdrawn when the item leaves the
//! queue, and is kept alive while shown so its click reaches the app.

use std::collections::HashMap;

use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use crate::identity;

const GROUP: &str = "decisions";

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Register the `AppUserModelID` with a display name, so toasts say "Tend"
/// (an unpackaged app has no manifest to say it).
pub fn register(app_id: &str) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{app_id}"))?;
    key.set_value("DisplayName", &identity::DISPLAY_NAME)?;
    Ok(())
}

pub fn unregister(app_id: &str) -> std::io::Result<()> {
    match RegKey::predef(HKEY_CURRENT_USER)
        .delete_subkey_all(format!(r"Software\Classes\AppUserModelId\{app_id}"))
    {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The toasts on screen, by tag.
pub struct Toasts {
    app_id: String,
    shown: HashMap<String, ToastNotification>,
}

impl Toasts {
    #[must_use]
    pub fn new(app_id: String) -> Self {
        Self {
            app_id,
            shown: HashMap::new(),
        }
    }

    /// Show one toast; `on_click` runs on a `WinRT` thread when it is clicked.
    pub fn show(
        &mut self,
        tag: &str,
        title: &str,
        body: &str,
        attribution: &str,
        on_click: impl Fn() + Send + 'static,
    ) -> windows::core::Result<()> {
        let xml = format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text><text placement=\"attribution\">{}</text></binding></visual></toast>",
            escape(title),
            escape(body),
            escape(attribution)
        );
        let document = XmlDocument::new()?;
        document.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&document)?;
        toast.SetTag(&HSTRING::from(tag))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
        toast.Activated(&TypedEventHandler::new(move |_, _| {
            on_click();
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(self.app_id.as_str()))?
            .Show(&toast)?;
        self.shown.insert(tag.to_owned(), toast);
        Ok(())
    }

    /// Withdraw a toast from the screen and the notification center.
    pub fn withdraw(&mut self, tag: &str) {
        self.shown.remove(tag);
        if let Ok(history) = ToastNotificationManager::History() {
            let _ = history.RemoveGroupedTagWithId(
                &HSTRING::from(tag),
                &HSTRING::from(GROUP),
                &HSTRING::from(self.app_id.as_str()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_text_cannot_add_markup() {
        assert_eq!(
            escape("<action content='x'/> & \"q\"\n"),
            "&lt;action content=&apos;x&apos;/&gt; &amp; &quot;q&quot; "
        );
    }
}
