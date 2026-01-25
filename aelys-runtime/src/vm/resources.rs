use super::VM;
use crate::stdlib::Resource;

impl VM {
    pub fn store_resource(&mut self, resource: Resource) -> usize {
        // reuse freed slot if available
        for (i, slot) in self.resources.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(resource);
                return i;
            }
        }
        // No free slot, append.
        let handle = self.resources.len();
        self.resources.push(Some(resource));
        handle
    }

    /// Get a resource by handle.
    pub fn get_resource(&self, handle: usize) -> Option<&Resource> {
        self.resources.get(handle)?.as_ref()
    }

    /// Get a mutable resource by handle.
    pub fn get_resource_mut(&mut self, handle: usize) -> Option<&mut Resource> {
        self.resources.get_mut(handle)?.as_mut()
    }

    /// Remove and return a resource.
    pub fn take_resource(&mut self, handle: usize) -> Option<Resource> {
        if handle < self.resources.len() {
            self.resources[handle].take()
        } else {
            None
        }
    }

    /// Check if a handle is valid.
    pub fn is_valid_handle(&self, handle: usize) -> bool {
        self.resources
            .get(handle)
            .map(|r| r.is_some())
            .unwrap_or(false)
    }
}
