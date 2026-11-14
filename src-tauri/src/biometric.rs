//! OS biometric prompt used to unlock the vault: Touch ID on macOS. Other systems report it
//! as unavailable and the vault is unlocked with the master password.

use crate::error::Res;

/// Returned when the person dismisses the prompt, so the UI can stay quiet about it.
pub const CANCELLED: &str = "cancelled";

/// Whether this computer can show a Touch ID prompt right now (hardware present and a finger
/// enrolled).
pub fn available() -> bool {
    imp::available()
}

/// Show the system prompt with `reason`. Resolves once the person has answered it.
pub async fn verify(reason: &str) -> Res<()> {
    imp::start(reason)?.await.map_err(|_| "Touch ID did not answer".to_string())?
}

#[cfg(target_os = "macos")]
mod imp {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAError, LAPolicy};
    use parking_lot::Mutex;
    use tokio::sync::oneshot;

    use crate::error::Res;

    // Biometrics only: falling back to the Mac login password is left to K-Shell's own
    // master password field.
    const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

    pub fn available() -> bool {
        let ctx = unsafe { LAContext::new() };
        unsafe { ctx.canEvaluatePolicy_error(POLICY) }.is_ok()
    }

    fn message(err: &NSError) -> String {
        match LAError(err.code()) {
            LAError::UserCancel | LAError::SystemCancel | LAError::AppCancel | LAError::UserFallback => {
                super::CANCELLED.into()
            }
            _ => err.localizedDescription().to_string(),
        }
    }

    /// Start the prompt; the receiver gets its answer. Kept synchronous so no Objective-C
    /// object is held across an `.await`.
    pub fn start(reason: &str) -> Res<oneshot::Receiver<Res<()>>> {
        // A context is single-use: a reused one lets a past success skip the prompt.
        let ctx = unsafe { LAContext::new() };
        unsafe { ctx.canEvaluatePolicy_error(POLICY) }.map_err(|e| message(&e))?;
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        // The context must outlive the reply, so the block keeps a reference to it.
        let keep = ctx.clone();
        let reply = RcBlock::new(move |ok: Bool, err: *mut NSError| {
            let _ = &keep;
            let res = if ok.as_bool() {
                Ok(())
            } else if err.is_null() {
                Err("Touch ID failed".to_string())
            } else {
                Err(message(unsafe { &*err }))
            };
            if let Some(tx) = tx.lock().take() {
                let _ = tx.send(res);
            }
        });
        unsafe { ctx.evaluatePolicy_localizedReason_reply(POLICY, &NSString::from_str(reason), &reply) };
        Ok(rx)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use crate::error::Res;
    use tokio::sync::oneshot;

    pub fn available() -> bool {
        false
    }

    pub fn start(_reason: &str) -> Res<oneshot::Receiver<Res<()>>> {
        Err("Touch ID is only available on macOS".into())
    }
}
