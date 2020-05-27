use crate::group::GroupContext;
use crate::key::PublicKey;
use crate::keypackage::{CipherSuite, KeyPackage, ProtocolVersion};
use crate::utils::{
    decode_option, encode_option, encode_vec_u32, encode_vec_u8_u16, encode_vec_u8_u8,
    read_vec_u32, read_vec_u8_u16, read_vec_u8_u8,
};
use rustls::internal::msgs::codec::{self, Codec, Reader};

/// spec: draft-ietf-mls-protocol.md#Add
#[derive(Debug, Clone)]
pub struct Add {
    pub key_package: KeyPackage,
}

/// spec: draft-ietf-mls-protocol.md#Update
#[derive(Debug, Clone)]
pub struct Update {
    pub key_package: KeyPackage,
}

/// spec: draft-ietf-mls-protocol.md#Remove
#[derive(Debug, Clone)]
pub struct Remove {
    pub removed: u32,
}

/// spec: draft-ietf-mls-protocol.md#Proposal
/// #[repr(u8)]
#[derive(Debug, Clone)]
pub enum Proposal {
    // Invalid = 0,
    Add(Add),       // = 1,
    Update(Update), // = 2,
    Remove(Remove), // = 3,
}

impl Codec for Proposal {
    fn encode(&self, bytes: &mut Vec<u8>) {
        match self {
            Proposal::Add(Add { key_package }) => {
                1u8.encode(bytes);
                key_package.encode(bytes);
            }
            Proposal::Update(Update { key_package }) => {
                2u8.encode(bytes);
                key_package.encode(bytes);
            }
            Proposal::Remove(Remove { removed }) => {
                3u8.encode(bytes);
                removed.encode(bytes);
            }
        }
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let tag = u8::read(r)?;
        match tag {
            1 => {
                let key_package = KeyPackage::read(r)?;
                Some(Proposal::Add(Add { key_package }))
            }
            2 => {
                let key_package = KeyPackage::read(r)?;
                Some(Proposal::Update(Update { key_package }))
            }
            3 => {
                let removed = u32::read(r)?;
                Some(Proposal::Remove(Remove { removed }))
            }
            _ => None,
        }
    }
}

/// spec: draft-ietf-mls-protocol.md#Message-Framing
/// + draft-ietf-mls-protocol.md#MContent-Signing-and-Encryption
#[derive(Debug, Clone)]
pub struct MLSPlaintextCommon {
    /// 0..255 bytes -- application-defined id
    pub group_id: Vec<u8>,
    /// version of the group key
    /// (incremented by 1 for each Commit message
    /// that is processed)
    pub epoch: u64,
    pub sender: Sender,
    /// 0..2^32-1
    pub authenticated_data: Vec<u8>,
    pub content: ContentType,
}

impl Codec for MLSPlaintextCommon {
    fn encode(&self, bytes: &mut Vec<u8>) {
        encode_vec_u8_u8(bytes, &self.group_id);
        self.epoch.encode(bytes);
        self.sender.encode(bytes);
        encode_vec_u32(bytes, &self.authenticated_data);
        match &self.content {
            ContentType::Application { application_data } => {
                1u8.encode(bytes);
                encode_vec_u32(bytes, &application_data);
            }
            ContentType::Proposal(p) => {
                2u8.encode(bytes);
                p.encode(bytes);
            }
            ContentType::Commit {
                commit,
                confirmation,
            } => {
                3u8.encode(bytes);
                commit.encode(bytes);
                encode_vec_u8_u8(bytes, confirmation);
            }
        }
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let group_id = read_vec_u8_u8(r)?;
        let epoch = u64::read(r)?;
        let sender = Sender::read(r)?;
        let authenticated_data: Vec<u8> = read_vec_u32(r)?;
        let tag = u8::read(r)?;
        let content = match tag {
            1 => {
                let application_data: Vec<u8> = read_vec_u32(r)?;
                Some(ContentType::Application { application_data })
            }
            2 => {
                let proposal = Proposal::read(r)?;
                Some(ContentType::Proposal(proposal))
            }
            3 => {
                let commit = Commit::read(r)?;
                let confirmation = read_vec_u8_u8(r)?;
                Some(ContentType::Commit {
                    commit,
                    confirmation,
                })
            }
            _ => None,
        }?;
        Some(MLSPlaintextCommon {
            group_id,
            epoch,
            sender,
            authenticated_data,
            content,
        })
    }
}

/// spec: draft-ietf-mls-protocol.md#Message-Framing\
#[derive(Debug)]
pub struct MLSPlaintext {
    pub content: MLSPlaintextCommon,
    /// 0..2^16-1
    pub signature: Vec<u8>,
}

impl Codec for MLSPlaintext {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.content.encode(bytes);
        encode_vec_u8_u16(bytes, &self.signature);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let content = MLSPlaintextCommon::read(r)?;
        let signature = read_vec_u8_u16(r)?;
        Some(MLSPlaintext { content, signature })
    }
}

impl MLSPlaintext {
    pub fn verify_signature(
        &self,
        context: &GroupContext,
        public_key: &PublicKey,
    ) -> Result<(), ring::error::Unspecified> {
        let payload = MLSPlaintextTBS {
            context: context.clone(),
            content: self.content.clone(),
        }
        .get_encoding();
        public_key.verify_signature(&payload, &self.signature)
    }
}

/// payload to be signed
/// spec: draft-ietf-mls-protocol.md#Content-Signing-and-Encryption
#[derive(Debug)]
pub struct MLSPlaintextTBS {
    /// TODO: https://github.com/mlswg/mls-protocol/issues/323 may be removed?
    pub context: GroupContext,
    pub content: MLSPlaintextCommon,
}

impl Codec for MLSPlaintextTBS {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.context.encode(bytes);
        self.content.encode(bytes);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let context = GroupContext::read(r)?;
        let content = MLSPlaintextCommon::read(r)?;
        Some(MLSPlaintextTBS { context, content })
    }
}

impl MLSPlaintext {
    pub fn get_add_keypackage(&self) -> Option<KeyPackage> {
        match &self.content.content {
            ContentType::Proposal(Proposal::Add(Add { key_package })) => Some(key_package.clone()),
            _ => None,
        }
    }
}

/// 0..255 -- hash of the MLSPlaintext in which the Proposal was sent
/// spec: draft-ietf-mls-protocol.md#Commit
#[derive(Debug, Clone)]
pub struct ProposalId(pub Vec<u8>);

impl Codec for ProposalId {
    fn encode(&self, bytes: &mut Vec<u8>) {
        encode_vec_u8_u8(bytes, &self.0);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let pid = read_vec_u8_u8(r)?;
        Some(ProposalId(pid))
    }
}

/// spec: draft-ietf-mls-protocol.md#Commit
#[derive(Debug, Clone)]
pub struct Commit {
    /// 0..2^16-1
    pub updates: Vec<ProposalId>,
    /// 0..2^16-1
    pub removes: Vec<ProposalId>,
    /// 0..2^16-1
    pub adds: Vec<ProposalId>,
    /// "path field of a Commit message MUST be populated if the Commit covers at least one Update or Remove proposal"
    /// "path field MUST also be populated if the Commit covers no proposals at all (i.e., if all three proposal vectors are empty)."
    pub path: Option<DirectPath>,
}

impl Codec for Commit {
    fn encode(&self, bytes: &mut Vec<u8>) {
        codec::encode_vec_u16(bytes, &self.updates);
        codec::encode_vec_u16(bytes, &self.removes);
        codec::encode_vec_u16(bytes, &self.adds);
        encode_option(bytes, &self.path);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let updates: Vec<ProposalId> = codec::read_vec_u16(r)?;
        let removes: Vec<ProposalId> = codec::read_vec_u16(r)?;
        let adds: Vec<ProposalId> = codec::read_vec_u16(r)?;
        let path: Option<DirectPath> = decode_option(r)?;

        Some(Commit {
            updates,
            removes,
            adds,
            path,
        })
    }
}

/// spec: draft-ietf-mls-protocol.md#Welcoming-New-Members
pub struct Welcome {
    pub version: ProtocolVersion,
    pub cipher_suite: CipherSuite,
    /// 0..2^32-1
    pub secrets: Vec<EncryptedGroupSecrets>,
    /// 0..2^32-1
    pub encrypted_group_info: Vec<u8>,
}

/// spec: draft-ietf-mls-protocol.md#Welcoming-New-Members
pub struct EncryptedGroupSecrets {
    pub encrypted_group_secrets: HPKECiphertext,
    pub key_package_hash: Vec<u8>,
}

/// spec: draft-ietf-mls-protocol.md#Direct-Paths
#[derive(Debug, Clone)]
pub struct HPKECiphertext {
    /// 0..2^16-1
    pub kem_output: Vec<u8>,
    /// 0..2^16-1
    pub ciphertext: Vec<u8>,
}

impl Codec for HPKECiphertext {
    fn encode(&self, bytes: &mut Vec<u8>) {
        encode_vec_u8_u16(bytes, &self.kem_output);
        encode_vec_u8_u16(bytes, &self.ciphertext);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let kem_output = read_vec_u8_u16(r)?;
        let ciphertext = read_vec_u8_u16(r)?;

        Some(HPKECiphertext {
            kem_output,
            ciphertext,
        })
    }
}

/// spec: draft-ietf-mls-protocol.md#Direct-Paths
#[derive(Debug, Clone)]
pub struct DirectPathNode {
    pub public_key: PublicKey,
    /// 0..2^32-1
    pub encrypted_path_secret: Vec<HPKECiphertext>,
}

impl Codec for DirectPathNode {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.public_key.encode(bytes);
        encode_vec_u32(bytes, &self.encrypted_path_secret);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let public_key = PublicKey::read(r)?;
        let encrypted_path_secret: Vec<HPKECiphertext> = read_vec_u32(r)?;

        Some(DirectPathNode {
            public_key,
            encrypted_path_secret,
        })
    }
}

/// spec: draft-ietf-mls-protocol.md#Direct-Paths
#[derive(Debug, Clone)]
pub struct DirectPath {
    pub leaf_key_package: KeyPackage,
    /// 0..2^16-1
    pub nodes: Vec<DirectPathNode>,
}

impl Codec for DirectPath {
    fn encode(&self, bytes: &mut Vec<u8>) {
        self.leaf_key_package.encode(bytes);
        codec::encode_vec_u16(bytes, &self.nodes);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let leaf_key_package = KeyPackage::read(r)?;
        let nodes: Vec<DirectPathNode> = codec::read_vec_u16(r)?;

        Some(DirectPath {
            leaf_key_package,
            nodes,
        })
    }
}

/// spec: draft-ietf-mls-protocol.md#Message-Framing
/// #[repr(u8)]
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum ContentType {
    Application {
        // <0..2^32-1>
        application_data: Vec<u8>,
    }, //= 1,
    Proposal(Proposal), //= 2,
    Commit {
        commit: Commit,
        // 0..255
        confirmation: Vec<u8>,
    }, //= 3,
}

/// spec: draft-ietf-mls-protocol.md#Message-Framing
#[repr(u8)]
#[derive(Debug, Copy, Clone)]
pub enum SenderType {
    Member = 1,
    Preconfigured = 2,
    NewMember = 3,
}

/// spec: draft-ietf-mls-protocol.md#Message-Framing
#[derive(Debug, Clone)]
pub struct Sender {
    pub sender_type: SenderType,
    pub sender: u32,
}

impl Codec for Sender {
    fn encode(&self, bytes: &mut Vec<u8>) {
        (self.sender_type as u8).encode(bytes);
        self.sender.encode(bytes);
    }

    fn read(r: &mut Reader) -> Option<Self> {
        let sender_t = u8::read(r)?;
        let sender_type = match sender_t {
            1 => Some(SenderType::Member),
            2 => Some(SenderType::Preconfigured),
            3 => Some(SenderType::NewMember),
            _ => None,
        }?;
        let sender = u32::read(r)?;
        Some(Self {
            sender_type,
            sender,
        })
    }
}
