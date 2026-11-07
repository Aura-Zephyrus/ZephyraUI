//! Extension execution report and statistics.

use std::fmt;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtensionReport {
    pub custom_passes: u32,
    pub offscreen_custom_passes: u32,
    pub unhandled_custom: u32,
    pub unsupported_offscreen_custom: u32,
    pub device_rebuilds: u32,
    pub screenshots: u32,
    pub readbacks: u32,
}

impl ExtensionReport {
    pub fn absorb(&mut self, other: ExtensionReport) {
        self.custom_passes += other.custom_passes;
        self.offscreen_custom_passes += other.offscreen_custom_passes;
        self.unhandled_custom += other.unhandled_custom;
        self.unsupported_offscreen_custom += other.unsupported_offscreen_custom;
        self.device_rebuilds += other.device_rebuilds;
        self.screenshots += other.screenshots;
        self.readbacks += other.readbacks;
    }
    pub fn is_empty(&self) -> bool {
        self.custom_passes == 0
            && self.offscreen_custom_passes == 0
            && self.unhandled_custom == 0
            && self.unsupported_offscreen_custom == 0
            && self.device_rebuilds == 0
            && self.screenshots == 0
            && self.readbacks == 0
    }
    pub fn summary(&self) -> Vec<(&'static str, u32)> {
        let mut v = Vec::new();
        if self.custom_passes > 0 {
            v.push(("custom-passes", self.custom_passes));
        }
        if self.offscreen_custom_passes > 0 {
            v.push(("offscreen-custom-passes", self.offscreen_custom_passes));
        }
        if self.unhandled_custom > 0 {
            v.push(("unhandled-custom", self.unhandled_custom));
        }
        if self.unsupported_offscreen_custom > 0 {
            v.push((
                "unsupported-offscreen-custom",
                self.unsupported_offscreen_custom,
            ));
        }
        if self.device_rebuilds > 0 {
            v.push(("device-rebuilds", self.device_rebuilds));
        }
        if self.screenshots > 0 {
            v.push(("screenshots", self.screenshots));
        }
        if self.readbacks > 0 {
            v.push(("readbacks", self.readbacks));
        }
        v
    }
}

impl fmt::Display for ExtensionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts = self.summary();
        if parts.is_empty() {
            write!(f, "ExtensionReport(empty)")
        } else {
            write!(
                f,
                "ExtensionReport({})",
                parts
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

pub trait RenderReportExt {
    fn absorb_extension(&mut self, ext: ExtensionReport);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extension_report_absorb() {
        let mut a = ExtensionReport {
            custom_passes: 2,
            unhandled_custom: 1,
            ..Default::default()
        };
        let b = ExtensionReport {
            custom_passes: 3,
            offscreen_custom_passes: 1,
            screenshots: 1,
            ..Default::default()
        };
        a.absorb(b);
        assert_eq!(a.custom_passes, 5);
        assert_eq!(a.offscreen_custom_passes, 1);
        assert_eq!(a.unhandled_custom, 1);
        assert_eq!(a.screenshots, 1);
    }
    #[test]
    fn extension_report_empty() {
        let r = ExtensionReport::default();
        assert!(r.is_empty());
        assert_eq!(r.summary(), vec![]);
    }
    #[test]
    fn extension_report_display() {
        let r = ExtensionReport {
            custom_passes: 2,
            unhandled_custom: 1,
            ..Default::default()
        };
        let s = r.to_string();
        assert!(s.contains("custom-passes=2"));
        assert!(s.contains("unhandled-custom=1"));
    }
}
