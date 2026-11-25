//! Smacker's Huffman trees.
//!
//! A packed tree is a pre-order walk: a 1 bit is an internal node (its 0 branch follows, then
//! its 1 branch), a 0 bit is a leaf followed by the leaf's value. An 8-bit tree's leaves hold an
//! 8-bit value; a 16-bit ("big") tree's leaves hold a low byte and a high byte, each coded with
//! its own 8-bit tree. Three values of a big tree are escapes: a leaf carrying one of them
//! stands for one of the tree's three most recently decoded values instead.

use crate::Error;
use crate::bits::BitReader;

const MAX_BYTE_DEPTH: u32 = 32;
const MAX_BIG_DEPTH: u32 = 500;

const LUT_BITS: u32 = 8;
const LUT_SIZE: usize = 1 << LUT_BITS;

/// Leaf bit in a node or lookup-table word.
const LEAF: u32 = 1 << 31;
/// Code length (in bits, 0..=LUT_BITS) inside a leaf lookup-table word.
const LEN_SHIFT: u32 = 24;
const VALUE_MASK: u32 = (1 << 18) - 1;
/// In a big tree's leaf, bits 16..18 hold 1 + the index of the recent value it stands for.
const RECENT_SHIFT: u32 = 16;

/// A decoding tree. Node words: a leaf is `LEAF | value`; an internal node's 0 branch is the
/// next node and its word is the index of its 1 branch. An empty tree decodes to 0 and reads
/// no bits; so does a tree that is a single leaf.
#[derive(Clone, Debug)]
struct Tree {
    nodes: Vec<u32>,
    lut: Box<[u32; LUT_SIZE]>,
}

impl Default for Tree {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            lut: Box::new([LEAF; LUT_SIZE]),
        }
    }
}

impl Tree {
    /// Rebuilds a packed tree, mapping each leaf value through `leaf`.
    fn read(
        br: &mut BitReader,
        max_depth: u32,
        leaf: &mut dyn FnMut(&mut BitReader) -> u32,
    ) -> Result<Tree, Error> {
        let mut tree = Tree::default();
        tree.read_node(br, max_depth, leaf, 0)?;
        if !tree.nodes.is_empty() {
            let mut lut = Box::new([LEAF; LUT_SIZE]);
            tree.fill_lut(&mut lut, 0, 0, 0);
            tree.lut = lut;
        }
        Ok(tree)
    }

    fn read_node(
        &mut self,
        br: &mut BitReader,
        max_depth: u32,
        leaf: &mut dyn FnMut(&mut BitReader) -> u32,
        depth: u32,
    ) -> Result<(), Error> {
        if depth > max_depth || br.overrun() {
            return Err(Error::Corrupt("Huffman tree too deep or truncated"));
        }
        if br.bit() {
            let me = self.nodes.len();
            self.nodes.push(0);
            self.read_node(br, max_depth, leaf, depth + 1)?;
            self.nodes[me] = self.nodes.len() as u32;
            self.read_node(br, max_depth, leaf, depth + 1)
        } else {
            let v = leaf(br);
            self.nodes.push(LEAF | v);
            Ok(())
        }
    }

    fn fill_lut(&self, lut: &mut [u32; LUT_SIZE], idx: usize, code: usize, depth: u32) {
        let n = self.nodes[idx];
        if n & LEAF != 0 {
            let entry = LEAF | (depth << LEN_SHIFT) | (n & VALUE_MASK);
            let step = 1usize << depth;
            let mut i = code;
            while i < LUT_SIZE {
                lut[i] = entry;
                i += step;
            }
        } else if depth == LUT_BITS {
            lut[code] = idx as u32;
        } else {
            self.fill_lut(lut, idx + 1, code, depth + 1);
            self.fill_lut(lut, n as usize, code | (1 << depth), depth + 1);
        }
    }

    #[inline]
    fn decode(&self, br: &mut BitReader) -> u32 {
        let entry = self.lut[br.peek(LUT_BITS) as usize];
        if entry & LEAF != 0 {
            br.consume((entry >> LEN_SHIFT) & 0x7f);
            return entry & VALUE_MASK;
        }
        br.consume(LUT_BITS);
        let mut i = entry as usize;
        loop {
            let Some(&n) = self.nodes.get(i) else {
                return 0;
            };
            if n & LEAF != 0 {
                return n & !LEAF;
            }
            i = if br.bit() { n as usize } else { i + 1 };
        }
    }
}

/// An 8-bit tree as stored: a presence bit, then (if set) the packed tree and a closing 0 bit.
#[derive(Clone, Debug, Default)]
pub(crate) struct ByteTree {
    tree: Tree,
}

impl ByteTree {
    pub(crate) fn read(br: &mut BitReader) -> Result<ByteTree, Error> {
        if !br.bit() {
            return Ok(ByteTree::default());
        }
        let tree = Tree::read(br, MAX_BYTE_DEPTH, &mut |br| br.bits(8))?;
        br.bit();
        Ok(ByteTree { tree })
    }

    #[inline]
    pub(crate) fn decode(&self, br: &mut BitReader) -> u8 {
        self.tree.decode(br) as u8
    }
}

/// A 16-bit tree with its three recent values (the video's MMap, MClr, Full and Type tables).
#[derive(Clone, Debug, Default)]
pub(crate) struct BigTree {
    tree: Tree,
    recent: [u16; 3],
}

impl BigTree {
    /// Stored as: a presence bit; then the low-byte tree, the high-byte tree, the three 16-bit
    /// escape values, the packed big tree and a closing 0 bit.
    pub(crate) fn read(br: &mut BitReader) -> Result<BigTree, Error> {
        if !br.bit() {
            return Ok(BigTree::default());
        }
        let lo = ByteTree::read(br)?;
        let hi = ByteTree::read(br)?;
        let escapes = [br.bits(16), br.bits(16), br.bits(16)];
        let mut marked = [false; 3];
        let tree = Tree::read(br, MAX_BIG_DEPTH, &mut |br| {
            let v = lo.decode(br) as u32 | (hi.decode(br) as u32) << 8;
            for (i, &e) in escapes.iter().enumerate() {
                if v == e && !marked[i] {
                    marked[i] = true;
                    return (i as u32 + 1) << RECENT_SHIFT;
                }
            }
            v
        })?;
        br.bit();
        Ok(BigTree {
            tree,
            recent: [0; 3],
        })
    }

    /// Forgets the recent values (done at the start of every frame).
    pub(crate) fn reset(&mut self) {
        self.recent = [0; 3];
    }

    #[inline]
    pub(crate) fn decode(&mut self, br: &mut BitReader) -> u16 {
        let leaf = self.tree.decode(br);
        let recent = leaf >> RECENT_SHIFT;
        let v = if recent != 0 {
            self.recent[recent as usize - 1]
        } else {
            leaf as u16
        };
        if self.recent[0] != v {
            self.recent[2] = self.recent[1];
            self.recent[1] = self.recent[0];
            self.recent[0] = v;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packs bits (each 0 or 1) least significant first.
    fn pack(bits: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; bits.len().div_ceil(8) + 8];
        for (i, &b) in bits.iter().enumerate() {
            out[i / 8] |= b << (i % 8);
        }
        out
    }

    fn byte(v: u8) -> Vec<u8> {
        (0..8).map(|i| (v >> i) & 1).collect()
    }

    #[test]
    fn byte_tree_round_trip() {
        // present, node(leaf 0x41, node(leaf 0x42, leaf 0x43)), closing 0;
        // then codes: 0 -> 0x41, 10 -> 0x42, 11 -> 0x43.
        let mut b = vec![1, 1, 0];
        b.extend(byte(0x41));
        b.extend([1, 0]);
        b.extend(byte(0x42));
        b.push(0);
        b.extend(byte(0x43));
        b.push(0);
        b.extend([1, 1, 0, 1, 0]);
        let data = pack(&b);
        let mut br = BitReader::new(&data);
        let t = ByteTree::read(&mut br).unwrap();
        assert_eq!(t.decode(&mut br), 0x43);
        assert_eq!(t.decode(&mut br), 0x41);
        assert_eq!(t.decode(&mut br), 0x42);
    }

    #[test]
    fn absent_and_single_leaf_trees_read_no_bits() {
        let data = pack(&[0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1]);
        let mut br = BitReader::new(&data);
        let empty = ByteTree::read(&mut br).unwrap();
        let single = ByteTree::read(&mut br).unwrap();
        assert_eq!(br.position(), 12);
        assert_eq!(empty.decode(&mut br), 0);
        assert_eq!(single.decode(&mut br), 0xff);
        assert_eq!(br.position(), 12);
    }

    #[test]
    fn big_tree_escapes_use_recent_values() {
        // Low tree: leaves 0x00 (code 0) and 0x34 (code 1). High tree: single leaf 0x12.
        let mut b = vec![1];
        b.extend([1, 1, 0]);
        b.extend(byte(0x00));
        b.push(0);
        b.extend(byte(0x34));
        b.push(0);
        b.extend([1, 0]);
        b.extend(byte(0x12));
        b.push(0);
        // Escapes: 0x1200 (recent[0]), and two unused values.
        for e in [0x1200u16, 0xaaaa, 0xbbbb] {
            b.extend((0..16).map(|i| ((e >> i) & 1) as u8));
        }
        // Big tree: node(leaf lo=0x00 -> escape 0, leaf lo=0x34 -> 0x1234), closing 0.
        b.extend([1, 0, 0, 0, 1, 0]);
        // Decode: 1 -> 0x1234, 0 -> recent[0] = 0x1234, 1, 0.
        b.extend([1, 0]);
        let data = pack(&b);
        let mut br = BitReader::new(&data);
        let mut t = BigTree::read(&mut br).unwrap();
        assert_eq!(t.decode(&mut br), 0x1234);
        assert_eq!(t.decode(&mut br), 0x1234);
        t.reset();
        assert_eq!(t.decode(&mut br), 0);
    }

    #[test]
    fn too_deep_tree_is_an_error() {
        let data = vec![0xffu8; 16];
        let mut br = BitReader::new(&data);
        assert!(ByteTree::read(&mut br).is_err());
    }

    #[test]
    fn big_tree_accepts_depth_over_32() {
        let depth = 40usize;
        // Low tree: 0 -> 0x11, 1 -> 0x22. High tree: single leaf 0x33.
        let mut b = vec![1];
        b.extend([1, 1, 0]);
        b.extend(byte(0x11));
        b.push(0);
        b.extend(byte(0x22));
        b.push(0);
        b.extend([1, 0]);
        b.extend(byte(0x33));
        b.push(0);
        for e in [0xaaaa_u16, 0xbbbb, 0xcccc] {
            b.extend((0..16).map(|i| ((e >> i) & 1) as u8));
        }
        // Right-skewed tree of depth 40: each level is internal(leaf 0x3311, subtree), ending in leaf 0x3322.
        for _ in 0..depth {
            b.extend([1, 0, 0]);
        }
        b.extend([0, 1]);
        b.push(0);
        // Decode the deepest leaf: `depth` ones.
        b.extend(std::iter::repeat_n(1u8, depth));
        // Decode the shallowest leaf: single 0.
        b.push(0);
        let data = pack(&b);
        let mut br = BitReader::new(&data);
        let mut t = BigTree::read(&mut br).unwrap();
        assert_eq!(t.decode(&mut br), 0x3322);
        assert_eq!(t.decode(&mut br), 0x3311);
        assert!(!br.overrun());
    }
}
