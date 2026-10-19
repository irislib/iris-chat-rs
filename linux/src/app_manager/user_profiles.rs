use super::*;

#[cfg(feature = "ui-tests")]
pub(super) type ProfileReader = std::rc::Rc<dyn Fn(&str) -> iris_chat_core::UserProfileSnapshot>;

impl AppManager {
    pub fn user_profile(&self, owner: &str) -> iris_chat_core::UserProfileSnapshot {
        #[cfg(feature = "ui-tests")]
        if let Some(reader) = self.profile_reader.borrow().as_ref() {
            return reader(owner);
        }
        self.ffi.user_profile(owner.to_owned())
    }

    #[cfg(feature = "ui-tests")]
    pub fn set_profile_reader(
        &self,
        reader: impl Fn(&str) -> iris_chat_core::UserProfileSnapshot + 'static,
    ) {
        *self.profile_reader.borrow_mut() = Some(std::rc::Rc::new(reader));
    }

    pub(crate) fn track_user_profile(&self, mut binding: crate::widgets::user_profile::Binding) {
        binding.refresh(self, &self.current_state());
        let mut bindings = self.user_profile_bindings.borrow_mut();
        bindings.retain(|binding| binding.is_alive());
        bindings.push(binding);
    }
}
