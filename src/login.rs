use objc2_service_management::SMAppServiceStatus;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoginState {
    On,
    Off,
    /// 以前「システム設定」でオフにされたなどで、利用者の承認が要る
    NeedsApproval,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Register,
    Unregister,
    OpenSettings,
}

pub fn state(status: SMAppServiceStatus) -> LoginState {
    if status == SMAppServiceStatus::Enabled {
        LoginState::On
    } else if status == SMAppServiceStatus::RequiresApproval {
        LoginState::NeedsApproval
    } else {
        LoginState::Off
    }
}

/// メニューの「ログイン時に起動」が選ばれたときにすること
pub fn action(state: LoginState) -> Action {
    match state {
        LoginState::On => Action::Unregister,
        LoginState::Off => Action::Register,
        LoginState::NeedsApproval => Action::OpenSettings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_service_status() {
        assert_eq!(state(SMAppServiceStatus::Enabled), LoginState::On);
        assert_eq!(state(SMAppServiceStatus::NotRegistered), LoginState::Off);
        assert_eq!(state(SMAppServiceStatus::NotFound), LoginState::Off);
        assert_eq!(
            state(SMAppServiceStatus::RequiresApproval),
            LoginState::NeedsApproval
        );
    }

    #[test]
    fn toggles_or_sends_to_settings() {
        assert_eq!(action(LoginState::On), Action::Unregister);
        assert_eq!(action(LoginState::Off), Action::Register);
        assert_eq!(action(LoginState::NeedsApproval), Action::OpenSettings);
    }
}
