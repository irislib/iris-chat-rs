use super::DesktopAudioDevice;
use cpal::traits::{DeviceTrait, HostTrait};
use std::collections::HashMap;

pub(super) struct Devices {
    devices: Vec<(DesktopAudioDevice, cpal::Device)>,
    default: Option<cpal::Device>,
    default_name: Option<String>,
}
impl Devices {
    pub(super) fn scan(host: &cpal::Host, input: bool) -> Self {
        let default = if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        };
        let devices: Vec<_> = host
            .devices()
            .ok()
            .into_iter()
            .flatten()
            .filter(|device| {
                if input {
                    device.supports_input()
                } else {
                    device.supports_output()
                }
            })
            .filter_map(|device| device.name().ok().map(|name| (name, device)))
            .collect();
        let descriptions = describe(devices.iter().map(|(name, _)| name.clone()));
        let default_name = default.as_ref().and_then(|device| device.name().ok());
        Self {
            default,
            default_name,
            devices: descriptions
                .into_iter()
                .zip(devices.into_iter().map(|(_, d)| d))
                .collect(),
        }
    }
    pub(super) fn options(&self) -> Vec<DesktopAudioDevice> {
        std::iter::once(DesktopAudioDevice {
            id: String::new(),
            name: self
                .default_name
                .as_ref()
                .map(|name| format!("System default ({name})"))
                .unwrap_or_else(|| "System default".into()),
        })
        .chain(self.devices.iter().map(|(info, _)| info.clone()))
        .collect()
    }
    pub(super) fn selected(&self, preferred: &str) -> (String, Option<&cpal::Device>) {
        if let Some((info, device)) = self.devices.iter().find(|(info, _)| info.id == preferred) {
            return (info.id.clone(), Some(device));
        }
        (String::new(), self.default.as_ref())
    }
    pub(super) fn fingerprint(&self, preferred: &str) -> String {
        if let Some((info, _)) = self.devices.iter().find(|(info, _)| info.id == preferred) {
            format!("device:{}", info.id)
        } else {
            format!(
                "default:{}",
                self.default_name.as_deref().unwrap_or_default()
            )
        }
    }
}

// CPAL 0.16 exposes names, not portable persistent identifiers. Keep these choices
// local to a call; a uniquely named device remains selected when enumeration reorders.
fn describe(names: impl Iterator<Item = String>) -> Vec<DesktopAudioDevice> {
    let mut counts = HashMap::<String, usize>::new();
    names
        .map(|name| {
            let occurrence = counts.entry(name.clone()).or_default();
            *occurrence += 1;
            DesktopAudioDevice {
                id: format!("{}:{name}:{}", name.len(), occurrence),
                name: if *occurrence == 1 {
                    name
                } else {
                    format!("{name} ({occurrence})")
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_identity_survives_unrelated_hotplug_and_duplicate_names_are_distinct() {
        let before = describe(["Headset", "Desk"].into_iter().map(String::from));
        let after = describe(
            ["USB", "Desk", "Headset", "Headset"]
                .into_iter()
                .map(String::from),
        );
        let headset = before.iter().find(|d| d.name == "Headset");
        assert_eq!(headset, after.iter().find(|d| d.name == "Headset"));
        assert_eq!(
            after
                .iter()
                .map(|d| &d.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
        assert!(after.iter().any(|d| d.name == "Headset (2)"));
    }
}
