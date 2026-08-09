//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//
// Sponsored by NLnet as part of the Next Generation Internet initiative.
// See: https://nlnet.nl/NGI0
//

use bytes::{Buf, BufMut, Bytes, BytesMut};
use derive_new::new;
use serde::{Deserialize, Serialize};

use crate::packet::error::{TlvDecodeError, TlvDecodeResult};
use crate::packet::iana::AreaProxyStlvType;
use crate::packet::tlv::{TLV_HDR_SIZE, tlv_encode_end, tlv_encode_start};
use crate::packet::SystemId;

/// Area Proxy System Identifier Sub-TLV (type 1).
///
/// Exactly 6 octets carrying the Area Proxy System ID (RFC 9666 §4.3.1).
#[derive(Clone, Debug, PartialEq)]
#[derive(new)]
#[derive(Deserialize, Serialize)]
pub struct AreaProxySystemIdStlv(pub SystemId);

impl AreaProxySystemIdStlv {
    const SIZE: u8 = 6;

    pub(crate) fn decode(
        stlv_len: u8,
        buf: &mut Bytes,
    ) -> TlvDecodeResult<Self> {
        if stlv_len != Self::SIZE {
            return Err(TlvDecodeError::InvalidLength(stlv_len));
        }
        let system_id = SystemId::decode(buf)?;
        Ok(AreaProxySystemIdStlv(system_id))
    }

    pub(crate) fn encode(&self, buf: &mut BytesMut) {
        let start_pos =
            tlv_encode_start(buf, AreaProxyStlvType::SystemId as u8);
        self.0.encode(buf);
        tlv_encode_end(buf, start_pos);
    }

    pub(crate) fn len(&self) -> usize {
        TLV_HDR_SIZE + Self::SIZE as usize
    }

    pub(crate) fn get(&self) -> &SystemId {
        &self.0
    }
}
