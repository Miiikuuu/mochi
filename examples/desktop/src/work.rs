//! Primary-character work pose, driven by task state rather than animation ticks.

#[derive(Default)]
pub struct Work {
    requested: bool,
    owned: bool,
}

impl Work {
    pub fn request(&mut self, active: bool) {
        self.requested = active;
    }

    pub fn transition(&mut self, current: &str, ready: bool) -> Option<&'static str> {
        if self.owned && current != "work" {
            self.owned = false;
        }
        if !self.requested {
            if self.owned && ready {
                self.owned = false;
                return Some("idle");
            }
            return None;
        }
        if ready && current == "idle" {
            self.owned = true;
            return Some("work");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_once_and_returns_to_idle_when_task_finishes() {
        let mut work = Work::default();
        work.request(true);
        assert_eq!(work.transition("idle", false), None);
        assert_eq!(work.transition("idle", true), Some("work"));
        assert_eq!(work.transition("work", true), None);
        work.request(false);
        assert_eq!(work.transition("work", true), Some("idle"));
        assert_eq!(work.transition("idle", true), None);
    }

    #[test]
    fn interaction_can_preempt_and_active_task_resumes_after_recovery() {
        let mut work = Work::default();
        work.request(true);
        assert_eq!(work.transition("idle", true), Some("work"));
        assert_eq!(work.transition("wave", true), None);
        assert_eq!(work.transition("idle", true), Some("work"));
    }

    #[test]
    fn stopping_task_does_not_cancel_manual_work_clip() {
        let mut work = Work::default();
        work.request(false);
        assert_eq!(work.transition("work", true), None);
    }
}
