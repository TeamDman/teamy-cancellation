use crate::cancellation_scope_kind::CancellationScopeKind;
use crate::cancellation_scope_record::CancellationScopeRecord;
use crate::cancellation_scope_status::CancellationScopeStatus;
use std::collections::HashMap;
#[cfg(feature = "async")]
use std::task::Waker;

/// Coordinates admission, owner closure, and protection under one mutex.
#[derive(Debug, Default)]
pub(crate) struct CancellationScopeTree {
    next_id: usize,
    records: HashMap<usize, CancellationScopeRecord>,
    #[cfg(feature = "async")]
    pending_wakers: Vec<Waker>,
}

impl CancellationScopeTree {
    pub(crate) fn insert(&mut self, parent: Option<usize>) -> usize {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("cancellation scope identifiers exhausted");
        self.records
            .insert(id, CancellationScopeRecord::new(parent));
        id
    }

    /// The node itself may be pending; every owning ancestor must be active.
    fn ancestors_are_active(&self, id: usize) -> bool {
        let Some(record) = self.records.get(&id) else {
            return false;
        };
        if record.status == CancellationScopeStatus::Closed {
            return false;
        }
        let mut parent = record.parent;
        while let Some(id) = parent {
            let Some(record) = self.records.get(&id) else {
                return false;
            };
            if record.status != CancellationScopeStatus::Active {
                return false;
            }
            parent = record.parent;
        }
        true
    }

    fn lineage(&self, id: usize) -> Vec<usize> {
        let mut ids = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            let Some(record) = self.records.get(&id) else {
                break;
            };
            ids.push(id);
            current = record.parent;
        }
        ids
    }

    fn inherits_protection(&self, id: usize) -> bool {
        self.lineage(id)
            .into_iter()
            .any(|id| self.records[&id].protected)
    }

    pub(crate) fn admit(
        &mut self,
        id: usize,
        cancellation_requested: bool,
        kind: CancellationScopeKind,
    ) -> bool {
        if !self.ancestors_are_active(id) {
            self.close(id);
            return false;
        }
        if self.records[&id].status == CancellationScopeStatus::Active {
            return true;
        }
        if cancellation_requested
            && !kind.accepts_requested_cancellation()
            && !self.inherits_protection(id)
        {
            self.close(id);
            return false;
        }
        self.records.get_mut(&id).unwrap().status = CancellationScopeStatus::Active;
        if kind.is_protected() {
            self.records.get_mut(&id).unwrap().protected = true;
            for ancestor in self.lineage(id) {
                let record = self.records.get_mut(&ancestor).unwrap();
                record.retained = record
                    .retained
                    .checked_add(1)
                    .expect("cancellation scope protection count exhausted");
            }
        }
        true
    }

    pub(crate) fn is_stopping(&self, id: usize, cancellation_requested: bool) -> bool {
        if !self.ancestors_are_active(id) {
            return true;
        }
        cancellation_requested && !self.inherits_protection(id) && self.records[&id].retained == 0
    }

    /// Force closure also applies to inner completion and external owner drops.
    /// Such exits are allowed to abandon protected children. Releasing all
    /// descendant protections here prevents them retaining a surviving ancestor.
    pub(crate) fn close(&mut self, id: usize) -> bool {
        if !self.records.contains_key(&id) {
            return false;
        }
        let descendants: Vec<_> = self
            .records
            .keys()
            .copied()
            .filter(|candidate| {
                self.records[candidate].status != CancellationScopeStatus::Closed
                    && self.lineage(*candidate).contains(&id)
            })
            .collect();
        let changed = !descendants.is_empty();
        for descendant in descendants {
            let protected = {
                let record = self.records.get_mut(&descendant).unwrap();
                record.status = CancellationScopeStatus::Closed;
                // A saved closed scope must not retain the completed executor
                // task through its last registered waker. Transfer that waker
                // to notifications so it is also released outside the lock.
                #[cfg(feature = "async")]
                if let Some(waker) = record.waker.take() {
                    self.pending_wakers.push(waker);
                }
                std::mem::replace(&mut record.protected, false)
            };
            if protected {
                for ancestor in self.lineage(descendant) {
                    let record = self.records.get_mut(&ancestor).unwrap();
                    record.retained = record
                        .retained
                        .checked_sub(1)
                        .expect("cancellation scope protection count underflow");
                    // Becoming unretained can enable owner cancellation. Take
                    // the waker so repeated closures cannot busy-wake that task.
                    #[cfg(feature = "async")]
                    if record.retained == 0
                        && let Some(waker) = record.waker.take()
                    {
                        self.pending_wakers.push(waker);
                    }
                }
            }
        }
        changed
    }

    pub(crate) fn remove(&mut self, id: usize) -> bool {
        let changed = self.close(id);
        self.records.remove(&id).is_some() || changed
    }

    #[cfg(feature = "async")]
    pub(crate) fn register_waker(&mut self, id: usize, waker: Waker) -> Option<Waker> {
        if let Some(record) = self.records.get_mut(&id)
            && record.status != CancellationScopeStatus::Closed
            && record
                .waker
                .as_ref()
                .is_none_or(|old| !old.will_wake(&waker))
        {
            return record.waker.replace(waker);
        }
        // The caller's final state check sees a racing close; a stale
        // registration must not retain a completed executor task.
        Some(waker)
    }

    /// Transfer affected notification targets; callers wake after unlocking.
    #[cfg(feature = "async")]
    pub(crate) fn wakers(&mut self) -> Vec<Waker> {
        std::mem::take(&mut self.pending_wakers)
    }
}
