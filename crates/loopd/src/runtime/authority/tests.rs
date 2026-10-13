use std::sync::{
    Arc, OnceLock, TryLockError, Weak,
    atomic::{AtomicI64, Ordering},
};

use super::{Clock, Identity, Role, RuntimeAuthority, StoreError, StoreResult};
use crate::test_support::{FixtureClock, NOW};

struct GuardedClock {
    authority: OnceLock<Weak<RuntimeAuthority>>,
    value: AtomicI64,
}

impl Clock for GuardedClock {
    fn now_millis(&self) -> StoreResult<i64> {
        let authority = self
            .authority
            .get()
            .and_then(Weak::upgrade)
            .ok_or(StoreError::Unavailable("clock fixture"))?;
        // This observation fixes the sampling order without scheduler sleeps.
        // The old implementation samples while the watermark is unlocked.
        if !matches!(
            authority.last_observed.try_lock(),
            Err(TryLockError::WouldBlock)
        ) {
            return Err(StoreError::Corrupt("clock sample order"));
        }
        Ok(self.value.fetch_add(1, Ordering::SeqCst))
    }
}

fn authority(clock: Arc<dyn Clock>) -> RuntimeAuthority {
    RuntimeAuthority::new(
        vec![Identity {
            actor_id: "clock.worker".to_owned(),
            subject: "service:clock.worker".to_owned(),
            display_name: "Clock worker".to_owned(),
            role: Role::Research,
            certificate_sha256: vec![format!("sha256:{}", "01".repeat(32))],
            not_before_ms: NOW - 1_000,
            expires_at_ms: NOW + 60_000,
            run_ids: vec!["run.clock".to_owned()],
        }],
        vec![],
        clock,
    )
    .unwrap()
}

#[test]
fn serialized_clock() {
    let clock = Arc::new(GuardedClock {
        authority: OnceLock::new(),
        value: AtomicI64::new(NOW),
    });
    let authority = Arc::new(authority(clock.clone()));
    assert!(clock.authority.set(Arc::downgrade(&authority)).is_ok());
    // Check the order before spawning so the old implementation fails even if
    // concurrent readers happen not to overlap on a particular test host.
    assert_eq!(authority.now().unwrap(), NOW);
    let mut readings = std::thread::scope(|scope| {
        let workers = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    (0..32)
                        .map(|_| authority.now().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    readings.sort_unstable();
    assert_eq!(readings, ((NOW + 1)..=(NOW + 256)).collect::<Vec<_>>());
}

#[test]
fn clock_regression() {
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let authority = authority(clock.clone());
    assert_eq!(authority.now().unwrap(), NOW);
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(authority.now(), Err(StoreError::ClockRegression)));
    clock.0.store(NOW - 1, Ordering::SeqCst);
    assert!(matches!(authority.now(), Err(StoreError::ClockRegression)));
    clock.0.store(NOW, Ordering::SeqCst);
    assert_eq!(authority.now().unwrap(), NOW);
    clock.0.store(NOW + 1, Ordering::SeqCst);
    assert_eq!(authority.now().unwrap(), NOW + 1);
}
