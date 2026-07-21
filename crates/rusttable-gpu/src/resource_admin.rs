use super::{
    GpuResourcePool, PoolError, PoolEvent, ResourceId, ResourceLease, ResourceState, lock,
    release_id,
};

impl GpuResourcePool {
    #[must_use]
    pub fn lease_count(&self, id: ResourceId) -> usize {
        lock(&self.shared.state)
            .entries
            .get(&id)
            .map_or(0, |entry| entry.leases)
    }

    pub fn poison(&self, id: ResourceId) -> Result<(), PoolError> {
        let mut state = lock(&self.shared.state);
        let entry = state
            .entries
            .get_mut(&id)
            .ok_or(PoolError::UnknownResource(id))?;
        if matches!(entry.state, ResourceState::InFlight | ResourceState::Lost) {
            return Err(PoolError::InvalidTransition {
                id,
                state: entry.state,
                operation: "poison",
            });
        }
        entry.state = ResourceState::Poisoned;
        state.idle.remove(&id);
        state.events.push_back(PoolEvent::Poisoned(id));
        Ok(())
    }
}

impl ResourceLease {
    /// Removes a failed attempt's storage from the reusable pool.
    pub fn discard(mut self) -> Result<(), PoolError> {
        let id = self
            .id
            .as_ref()
            .copied()
            .ok_or(PoolError::DoubleReturn(self.original_id))?;
        {
            let mut state = lock(&self.pool.state);
            let entry = state
                .entries
                .get_mut(&id)
                .ok_or(PoolError::UnknownResource(id))?;
            if matches!(entry.state, ResourceState::InFlight | ResourceState::Lost)
                || entry.submission.is_some()
            {
                return Err(PoolError::InvalidTransition {
                    id,
                    state: entry.state,
                    operation: "discard before retire",
                });
            }
            entry.state = ResourceState::Poisoned;
            state.idle.remove(&id);
            state.events.push_back(PoolEvent::Poisoned(id));
        }
        release_id(&self.pool, id)?;
        self.id = None;
        Ok(())
    }
}
