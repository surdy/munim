//! Persisted settings + budget-alert state (BUILD_SPEC §5.2b, §0.5 #9/#10/#11).
//! Stored as settings.json in the app-config dir, separate from the session cache.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Monthly budget in USD; `None` = budget off (no bar, no alerts).
    #[serde(default)]
    pub monthly_budget: Option<f64>,
    /// Mirrors the OS autostart state; off by default (BUILD_SPEC §0.5 #9).
    #[serde(default)]
    pub launch_at_login: bool,
    /// Which budget thresholds have already alerted this month, so each fires once.
    /// Reset when `alert_month` changes. BUILD_SPEC §5.2b.
    #[serde(default)]
    pub alert_month: String, // "YYYY-MM"
    #[serde(default)]
    pub alerted_80: bool,
    #[serde(default)]
    pub alerted_100: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            monthly_budget: None,
            launch_at_login: false,
            alert_month: String::new(),
            alerted_80: false,
            alerted_100: false,
        }
    }
}

impl Settings {
    // TODO(spec §5.2b): load()/save() against <app-config>/settings.json (atomic write).

    /// Roll over the alert flags when the calendar month changes.
    pub fn ensure_month(&mut self, current_month: &str) {
        if self.alert_month != current_month {
            self.alert_month = current_month.to_string();
            self.alerted_80 = false;
            self.alerted_100 = false;
        }
    }

    /// Decide which (if any) budget notification to fire this collect. Returns the
    /// threshold crossed for the first time this month, or None. Caller fires the
    /// notification + persists. BUILD_SPEC §5.2b.
    pub fn budget_alert(&mut self, month_spend: f64, current_month: &str) -> Option<u8> {
        let budget = self.monthly_budget?;
        if budget <= 0.0 {
            return None;
        }
        self.ensure_month(current_month);
        let pct = month_spend / budget;
        if pct >= 1.0 && !self.alerted_100 {
            self.alerted_100 = true;
            self.alerted_80 = true; // crossing 100 implies 80 already covered
            return Some(100);
        }
        if pct >= 0.8 && !self.alerted_80 {
            self.alerted_80 = true;
            return Some(80);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_fire_once_then_rollover() {
        let mut s = Settings {
            monthly_budget: Some(100.0),
            ..Default::default()
        };
        assert_eq!(s.budget_alert(80.0, "2026-07"), Some(80));
        assert_eq!(s.budget_alert(85.0, "2026-07"), None); // 80 already fired
        assert_eq!(s.budget_alert(100.0, "2026-07"), Some(100));
        assert_eq!(s.budget_alert(120.0, "2026-07"), None);
        // new month resets
        assert_eq!(s.budget_alert(80.0, "2026-08"), Some(80));
    }

    #[test]
    fn no_budget_no_alert() {
        let mut s = Settings::default();
        assert_eq!(s.budget_alert(9999.0, "2026-07"), None);
    }
}
