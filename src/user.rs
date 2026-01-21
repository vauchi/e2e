//! User abstraction for E2E testing.
//!
//! Represents a Vauchi user with one or more linked devices.

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::device::{CliDevice, Contact, ContactCard, Device, DeviceType};
use crate::error::{E2eError, E2eResult};

/// A Vauchi user with one or more devices.
pub struct User {
    /// User's display name.
    name: String,
    /// All devices belonging to this user.
    devices: Vec<Arc<RwLock<Box<dyn Device>>>>,
    /// Index of the primary device (first device created).
    primary_device: usize,
    /// Default relay URL for this user's devices.
    relay_url: String,
}

impl User {
    /// Create a new user with no devices.
    pub fn new(name: impl Into<String>, relay_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            devices: Vec::new(),
            primary_device: 0,
            relay_url: relay_url.into(),
        }
    }

    /// Get the user's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the number of devices.
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Get a reference to a device by index.
    pub fn device(&self, index: usize) -> Option<&Arc<RwLock<Box<dyn Device>>>> {
        self.devices.get(index)
    }

    /// Get the primary device.
    pub fn primary_device(&self) -> Option<&Arc<RwLock<Box<dyn Device>>>> {
        self.devices.get(self.primary_device)
    }

    /// Get all devices.
    pub fn devices(&self) -> &[Arc<RwLock<Box<dyn Device>>>] {
        &self.devices
    }

    /// Add a CLI device to this user.
    pub fn add_cli_device(&mut self, device_name: impl Into<String>) -> E2eResult<usize> {
        let device_name = device_name.into();
        let full_name = format!("{}_{}", self.name, device_name);

        info!("Adding CLI device '{}' for user '{}'", device_name, self.name);

        let device = CliDevice::new(&full_name, &self.relay_url)?;
        let device: Box<dyn Device> = Box::new(device);
        self.devices.push(Arc::new(RwLock::new(device)));

        Ok(self.devices.len() - 1)
    }

    /// Add a device with a specific type (for future expansion).
    pub fn add_device(&mut self, device_name: impl Into<String>, device_type: DeviceType) -> E2eResult<usize> {
        match device_type {
            DeviceType::Cli => self.add_cli_device(device_name),
            _ => Err(E2eError::device(format!(
                "Device type {:?} not yet implemented",
                device_type
            ))),
        }
    }

    /// Add multiple CLI devices at once.
    pub fn add_cli_devices(&mut self, count: usize) -> E2eResult<Vec<usize>> {
        let mut indices = Vec::with_capacity(count);
        for i in 0..count {
            let name = format!("device_{}", i + 1);
            let index = self.add_cli_device(name)?;
            indices.push(index);
        }
        Ok(indices)
    }

    /// Create identity on the primary device.
    pub async fn create_identity(&self) -> E2eResult<()> {
        let primary = self.primary_device()
            .ok_or_else(|| E2eError::user("No devices available"))?;

        let device = primary.read().await;
        info!("Creating identity '{}' on primary device", self.name);
        device.create_identity(&self.name).await
    }

    /// Create identity on a specific device.
    pub async fn create_identity_on_device(&self, device_index: usize) -> E2eResult<()> {
        let device = self.device(device_index)
            .ok_or_else(|| E2eError::user(format!("Device {} not found", device_index)))?;

        let device = device.read().await;
        device.create_identity(&self.name).await
    }

    /// Link all secondary devices to the primary device.
    ///
    /// This performs the device linking flow:
    /// 1. Primary device generates link QR
    /// 2. Secondary device joins with the QR
    /// 3. Primary device completes the link
    /// 4. Secondary device finishes joining
    pub async fn link_devices(&self) -> E2eResult<()> {
        if self.devices.len() <= 1 {
            debug!("User '{}' has {} devices, no linking needed", self.name, self.devices.len());
            return Ok(());
        }

        let primary = self.primary_device()
            .ok_or_else(|| E2eError::user("No primary device"))?;

        for i in 1..self.devices.len() {
            info!("Linking device {} for user '{}'", i, self.name);

            // Step 1: Primary generates link QR
            let link_qr = {
                let device = primary.read().await;
                device.start_device_link().await?
            };

            // Step 2: Secondary joins with QR
            let secondary = &self.devices[i];
            let request_data = {
                let device = secondary.read().await;
                let device_name = format!("{}_{}", self.name, i);
                device.join_identity(&link_qr, &device_name).await?
            };

            // Step 3: Primary completes the link
            let response_data = {
                let device = primary.read().await;
                device.complete_device_link(&request_data).await?
            };

            // Step 4: Secondary finishes joining
            {
                let device = secondary.read().await;
                device.finish_device_join(&response_data).await?;
            }

            info!("Device {} linked successfully for user '{}'", i, self.name);
        }

        Ok(())
    }

    /// Sync all devices.
    pub async fn sync_all(&self) -> E2eResult<()> {
        for (i, device) in self.devices.iter().enumerate() {
            debug!("Syncing device {} for user '{}'", i, self.name);
            let device = device.read().await;
            device.sync().await?;
        }
        Ok(())
    }

    /// Sync a specific device.
    pub async fn sync_device(&self, device_index: usize) -> E2eResult<()> {
        let device = self.device(device_index)
            .ok_or_else(|| E2eError::user(format!("Device {} not found", device_index)))?;

        let device = device.read().await;
        device.sync().await
    }

    /// Generate exchange QR from the primary device.
    pub async fn generate_qr(&self) -> E2eResult<String> {
        let primary = self.primary_device()
            .ok_or_else(|| E2eError::user("No primary device"))?;

        let device = primary.read().await;
        device.generate_qr().await
    }

    /// Generate exchange QR from a specific device.
    pub async fn generate_qr_from_device(&self, device_index: usize) -> E2eResult<String> {
        let device = self.device(device_index)
            .ok_or_else(|| E2eError::user(format!("Device {} not found", device_index)))?;

        let device = device.read().await;
        device.generate_qr().await
    }

    /// Complete an exchange using the primary device.
    pub async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()> {
        let primary = self.primary_device()
            .ok_or_else(|| E2eError::user("No primary device"))?;

        let device = primary.read().await;
        device.complete_exchange(qr_data).await
    }

    /// Complete an exchange using a specific device.
    pub async fn complete_exchange_on_device(&self, device_index: usize, qr_data: &str) -> E2eResult<()> {
        let device = self.device(device_index)
            .ok_or_else(|| E2eError::user(format!("Device {} not found", device_index)))?;

        let device = device.read().await;
        device.complete_exchange(qr_data).await
    }

    /// List contacts on the primary device.
    pub async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        let primary = self.primary_device()
            .ok_or_else(|| E2eError::user("No primary device"))?;

        let device = primary.read().await;
        device.list_contacts().await
    }

    /// List contacts on a specific device.
    pub async fn list_contacts_on_device(&self, device_index: usize) -> E2eResult<Vec<Contact>> {
        let device = self.device(device_index)
            .ok_or_else(|| E2eError::user(format!("Device {} not found", device_index)))?;

        let device = device.read().await;
        device.list_contacts().await
    }

    /// Get the contact card from the primary device.
    pub async fn get_card(&self) -> E2eResult<ContactCard> {
        let primary = self.primary_device()
            .ok_or_else(|| E2eError::user("No primary device"))?;

        let device = primary.read().await;
        device.get_card().await
    }

    /// Add a field to the contact card on the primary device.
    pub async fn add_field(&self, field_type: &str, label: &str, value: &str) -> E2eResult<()> {
        let primary = self.primary_device()
            .ok_or_else(|| E2eError::user("No primary device"))?;

        let device = primary.read().await;
        device.add_field(field_type, label, value).await
    }

    /// Exchange contacts with another user.
    ///
    /// This user generates a QR and the other user completes the exchange.
    pub async fn exchange_with(&self, other: &User) -> E2eResult<()> {
        info!("User '{}' exchanging with '{}'", self.name, other.name);

        // This user generates QR
        let qr = self.generate_qr().await?;

        // Other user completes exchange
        other.complete_exchange(&qr).await?;

        // Both sync to complete the exchange
        self.sync_all().await?;
        other.sync_all().await?;

        Ok(())
    }

    /// Perform mutual exchange with another user.
    ///
    /// Both users exchange QRs and complete the exchange bidirectionally.
    pub async fn mutual_exchange_with(&self, other: &User) -> E2eResult<()> {
        info!("Mutual exchange between '{}' and '{}'", self.name, other.name);

        // This user generates QR and other completes
        let my_qr = self.generate_qr().await?;
        other.complete_exchange(&my_qr).await?;

        // Other generates QR and this user completes
        let their_qr = other.generate_qr().await?;
        self.complete_exchange(&their_qr).await?;

        // Both sync
        self.sync_all().await?;
        other.sync_all().await?;

        Ok(())
    }
}

/// Builder for creating users with specified device configurations.
pub struct UserBuilder {
    name: String,
    relay_url: String,
    device_count: usize,
    device_types: Vec<DeviceType>,
}

impl UserBuilder {
    /// Create a new user builder.
    pub fn new(name: impl Into<String>, relay_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            relay_url: relay_url.into(),
            device_count: 1,
            device_types: Vec::new(),
        }
    }

    /// Set the number of CLI devices.
    pub fn with_devices(mut self, count: usize) -> Self {
        self.device_count = count;
        self.device_types = vec![DeviceType::Cli; count];
        self
    }

    /// Set specific device types.
    pub fn with_device_types(mut self, types: Vec<DeviceType>) -> Self {
        self.device_count = types.len();
        self.device_types = types;
        self
    }

    /// Build the user.
    pub fn build(self) -> E2eResult<User> {
        let mut user = User::new(self.name, self.relay_url);

        if self.device_types.is_empty() {
            // Default to one CLI device
            user.add_cli_devices(self.device_count)?;
        } else {
            for (i, device_type) in self.device_types.into_iter().enumerate() {
                let name = format!("device_{}", i + 1);
                user.add_device(name, device_type)?;
            }
        }

        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_creation() {
        let user = User::new("Alice", "ws://localhost:8080");
        assert_eq!(user.name(), "Alice");
        assert_eq!(user.device_count(), 0);
    }

    #[test]
    #[ignore = "requires CLI binary to be built"]
    fn test_user_builder() {
        let user = UserBuilder::new("Bob", "ws://localhost:8080")
            .with_devices(3)
            .build()
            .unwrap();

        assert_eq!(user.name(), "Bob");
        assert_eq!(user.device_count(), 3);
    }

    #[test]
    fn test_user_builder_zero_devices() {
        let user = UserBuilder::new("Carol", "ws://localhost:8080")
            .with_devices(0)
            .build()
            .unwrap();

        assert_eq!(user.name(), "Carol");
        assert_eq!(user.device_count(), 0);
    }
}
