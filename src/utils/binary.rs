use super::board::BOARD_SIZE;

use {bitvec::prelude::*, halo2_proofs::arithmetic::FieldExt};

/**
 * Binary element with converstion functionality
 * @dev stored in 256 bit integer
 */
pub type U256 = BitArray<[u64; 4], Lsb0>; // 256 bit integer in little endian

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub struct BinaryValue {
    pub value: U256,
}

impl BinaryValue {
    // wrap 256 bit BitArray in BinaryValue object
    pub fn new(value: U256) -> BinaryValue {
        BinaryValue { value }
    }

    // return a 256 bit number from an 8 bit number
    pub fn from_u8(value: u8) -> BinaryValue {
        BinaryValue {
            value: U256::new([value as u64, 0, 0, 0]),
        }
    }

    // wrap an empty 256 bit BitArray in BinaryValue object
    pub fn empty() -> BinaryValue {
        BinaryValue::new(BitArray::ZERO)
    }

    // returns the u128 from first half of U256 in LE
    pub fn lower_u128(self) -> u128 {
        u128::from_le_bytes(
            self.value.into_inner()[0..2]
                .iter()
                .map(|value| value.to_le_bytes().to_vec())
                .flatten()
                .collect::<Vec<u8>>()
                .try_into()
                .unwrap(),
        )
    }

    /**
     * Convert the binary value to an array of bits on a given prime field
     *
     * @param F: the prime field to wrap bits in
     * @param S: the number of bits in the field
     * @return - array of bits of length S on Field F
     */
    pub fn bitfield<F: FieldExt, const S: usize>(self) -> [F; S] {
        self.value.into_inner().view_bits::<Lsb0>()[0..S]
            .into_iter()
            .map(|bit| F::from(*bit))
            .collect::<Vec<F>>()
            .try_into()
            .unwrap()
    }

    /**
     * Zip together bits
     * @dev fails if both bits set when trying to zip
     * @todo: better docs here im tired
     *
     * @return - BinaryValue where bits are zipped from two inputs
     */
    pub fn zip(self, to: BinaryValue) -> BinaryValue {
        let mut zipped = U256::ZERO;
        for i in 0..BOARD_SIZE {
            // only zip 100 bits
            if self.value[i] && to.value[i] {
                panic!("Cannot zip together bit #{}", i);
            };
            let bit = self.value[i] || to.value[i];
            zipped.set(i, bit);
        }
        BinaryValue::new(zipped)
    }
}
