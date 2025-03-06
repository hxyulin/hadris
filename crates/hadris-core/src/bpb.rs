//! This module contains structures and functions for working with the BIOS Parameter Block (BPB),
//! filesystem specific structures are not included.

/// The jump instruction for the start of the BPB
///
/// # Examples
/// ```
/// use hadris_core::bpb::JumpInstruction;
///
/// // Convert from bytes
/// let jump_instruction = JumpInstruction::from_bytes([0xEB, 0x01, 0x90]).unwrap();
/// // Convert to bytes
/// let bytes = jump_instruction.to_bytes();
/// assert_eq!(bytes, [0xEB, 0x01, 0x90]);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpInstruction {
    /// A short (relative) jump
    ShortJump(u8),
    /// A near (relative) jump
    NearJump(u16),
}

impl JumpInstruction {
    /// Converts a jump instruction to bytes
    ///
    /// # Errors
    /// This function will return an error if the jump instruction is not a short or near jump.
    pub fn from_bytes(bytes: [u8; 3]) -> Result<Self, ()> {
        if bytes[0] == 0xEB && bytes[2] == 0x90 {
            Ok(Self::ShortJump(bytes[1]))
        } else if bytes[0] == 0xE9 {
            Ok(Self::NearJump(u16::from_le_bytes(
                bytes[1..3].try_into().unwrap(),
            )))
        } else {
            Err(())
        }
    }

    /// Converts a jump instruction to bytes
    pub fn to_bytes(&self) -> [u8; 3] {
        match self {
            Self::ShortJump(byte) => [0xEB, *byte, 0x90],
            Self::NearJump(word) => [0xE9, word.to_le_bytes()[0], word.to_le_bytes()[1]],
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    // TODO(Testing): Add more tests based on x86 instruction set specification
    
    use super::*;

    #[test]
    fn test_jump_instruction_from_bytes() {
        assert_eq!(
            JumpInstruction::from_bytes([0xEB, 0x01, 0x90]).unwrap(),
            JumpInstruction::ShortJump(0x01)
        );
        assert_eq!(
            JumpInstruction::from_bytes([0xE9, 0x00, 0x01]).unwrap(),
            JumpInstruction::NearJump(0x0100)
        );
        assert!(JumpInstruction::from_bytes([0xEB, 0x01, 0x01]).is_err());
        assert!(JumpInstruction::from_bytes([0xEB, 0x01, 0x91]).is_err());
    }

    #[test]
    fn test_jump_instruction_to_bytes() {
        assert_eq!(
            JumpInstruction::ShortJump(0x01).to_bytes(),
            [0xEB, 0x01, 0x90]
        );
        assert_eq!(
            JumpInstruction::NearJump(0x0100).to_bytes(),
            [0xE9, 0x00, 0x01]
        );
    }
}
