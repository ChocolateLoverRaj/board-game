use bt_hci::param::BdAddr;
use defmt::Format;
use serde::{Deserialize, Serialize};
use trouble_host::{
    BondInformation, Identity, IdentityResolvingKey, LongTermKey, prelude::SecurityLevel,
};

#[derive(Debug, Format, Serialize, Deserialize, Clone)]
pub struct StoredBondInformation {
    pub ltk: u128,
    pub bd_addr: [u8; 6],
    pub irk: Option<u128>,
    pub security_level: u8,
}

impl From<StoredBondInformation> for BondInformation {
    fn from(value: StoredBondInformation) -> Self {
        Self {
            ltk: LongTermKey::new(value.ltk),
            identity: Identity {
                bd_addr: BdAddr::new(value.bd_addr),
                irk: value.irk.map(IdentityResolvingKey::new),
            },
            is_bonded: true,
            security_level: match value.security_level {
                0 => SecurityLevel::NoEncryption,
                1 => SecurityLevel::Encrypted,
                2 => SecurityLevel::EncryptedAuthenticated,
                _ => unreachable!(),
            },
        }
    }
}

impl From<BondInformation> for StoredBondInformation {
    fn from(value: BondInformation) -> Self {
        Self {
            ltk: value.ltk.0,
            bd_addr: value.identity.bd_addr.into_inner(),
            irk: value.identity.irk.map(|irk| irk.0),
            security_level: match value.security_level {
                SecurityLevel::NoEncryption => 0,
                SecurityLevel::Encrypted => 1,
                SecurityLevel::EncryptedAuthenticated => 2,
            },
        }
    }
}

pub const STORED_BONDS_LEN: usize = 10;

// Everything that's stored
#[derive(Debug, Format, Default, Serialize, Deserialize)]
pub struct LiberalStorage {
    pub last_connected_peripheral: Option<[u8; 6]>,
    pub saved_bonds: heapless::Vec<StoredBondInformation, STORED_BONDS_LEN>,
}

#[derive(Debug, Format, Default, Serialize, Deserialize)]
pub struct FascistStorage {
    pub saved_bonds: heapless::Vec<StoredBondInformation, STORED_BONDS_LEN>,
}

/// This is an estimate
pub const LIBERAL_DATA_BUFFER_LEN: usize = size_of::<LiberalStorage>();
pub const FASCIST_DATA_BUFFER_LEN: usize = size_of::<LiberalStorage>();
