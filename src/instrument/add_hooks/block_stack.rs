use self::BlockStackElement::*;
use std::collections::HashMap;
use wasm::ast::{highlevel::Instr, Idx, Label};

/*
 * Data structure for representing the "control stack", i.e., the implicit nested block structure
 * of control-flow instructions.
 * Needed for:
 *  - resolving End instructions to their corresponding begins (i.e., Block, Loop, If, Else)
 *  - resolving labels of branches to actual instruction indices (which requires the previous as a first step)
 */

#[derive(Debug)]
pub struct BlockStack {
    block_stack: Vec<BlockStackElement>,
    /// pre-computed on new()
    begin_end_map: HashMap<Idx<Instr>, Idx<Instr>>,
}

#[derive(Debug, Clone, Serialize)]
// see https://serde.rs/enum-representations.html
#[serde(rename_all = "lowercase", tag = "type")]
pub enum BlockStackElement {
    Function {
        end: Idx<Instr>,
    },
    Block {
        begin: Idx<Instr>,
        end: Idx<Instr>,
    },
    Loop {
        begin: Idx<Instr>,
        end: Idx<Instr>,
    },
    If {
        #[serde(rename = "begin")]
        begin_if: Idx<Instr>,
        begin_else: Option<Idx<Instr>>,
        end: Idx<Instr>,
    },
    Else {
        #[serde(rename = "begin")]
        begin_else: Idx<Instr>,
        begin_if: Idx<Instr>,
        end: Idx<Instr>,
    },
}

impl BlockStack {
    pub fn new(instrs: &[Instr]) -> Self {
        // build this already at construction, so that we know later in O(1) where the end's are
        let mut begin_end_map: HashMap<Idx<Instr>, Idx<Instr>> = HashMap::new();

        let mut begin_stack: Vec<Idx<Instr>> = vec![];
        for (iidx, instr) in instrs[..instrs.len() - 1].iter().enumerate() {
            let iidx = iidx.into();
            match *instr {
                Instr::Block(_) | Instr::Loop(_) | Instr::If(_) => begin_stack.push(iidx),
                Instr::Else | Instr::End => {
                    let begin_iidx = begin_stack.pop().expect("invalid block nesting: could not end block, stack was empty");
                    begin_end_map.insert(begin_iidx, iidx);
                    // special case: Else also starts its own block
                    if let Instr::Else = instr {
                        begin_stack.push(iidx);
                    }
                }
                _ => {}
            }
        }
        assert!(begin_stack.is_empty(), "invalid block nesting: some blocks were not closed, stack at end is {:?}", begin_stack);

        BlockStack {
            block_stack: vec![Function { end: (instrs.len() - 1).into() }],
            begin_end_map,
        }
    }

    pub fn begin_block(&mut self, begin: Idx<Instr>) {
        self.block_stack.push(Block {
            begin,
            end: *self.begin_end_map.get(&begin)
                .expect(&format!("invalid block nesting: could not find end for block begin at {:?}", begin)),
        });
    }

    pub fn begin_loop(&mut self, begin: Idx<Instr>) {
        self.block_stack.push(Loop {
            begin,
            end: *self.begin_end_map.get(&begin)
                .expect(&format!("invalid block nesting: could not find end for loop begin at {:?}", begin)),
        });
    }

    pub fn begin_if(&mut self, begin_if: Idx<Instr>) {
        let end_or_else = *self.begin_end_map.get(&begin_if)
            .expect(&format!("invalid block nesting: could not find end/else for if begin at {:?}", begin_if));

        let if_ = if let Some(&end) = self.begin_end_map.get(&end_or_else) {
            If {
                begin_if,
                begin_else: Some(end_or_else),
                end,
            }
        } else {
            If {
                begin_if,
                begin_else: None,
                end: end_or_else,
            }
        };
        self.block_stack.push(if_);
    }

    /// returns matching If block (of which this else is a "sibling")
    pub fn else_(&mut self) -> BlockStackElement {
        match self.block_stack.pop() {
            Some(block_element) => match block_element {
                If { begin_if, begin_else: Some(begin_else), end } => {
                    self.block_stack.push(Else { begin_if, begin_else, end });
                    block_element
                }
                block => panic!("invalid block nesting: expected if with else on block stack, but got {:?}", block),
            }
            None => panic!("invalid block nesting: expected if, but stack was empty"),
        }
    }

    pub fn end(&mut self) -> BlockStackElement {
        self.block_stack.pop().expect("invalid block nesting: could not end block, stack was empty")
    }

    /// resolves a relative label at the current instruction to an absolute instruction index
    /// this requires forward scanning for non-loop block ends (implemented as a precomputed HashMap lookup, so O(1))
    pub fn br_target(&self, label: Idx<Label>) -> BranchTarget {
        // resolve label to all blocks between the current and the target block
        let ended_blocks: Vec<BlockStackElement> = self.block_stack.iter().rev().take(label.0 + 1).cloned().collect();

        // resolve block begin to actual next instruction locations
        // backward branch when targeting loops, forward for all other blocks
        let absolute_instr = {
            // the last block of the ended ones is the actual target
            let target_block = ended_blocks.get(label.0)
                .expect(&format!("invalid label: cannot find target block for {:?}", label));

            match *target_block {
                Loop { begin, .. } => begin,
                Function { end } | Block { end, .. } | If { end, .. } | Else { end, .. } => end,
            }
        };

        BranchTarget { absolute_instr, ended_blocks }
    }

    /// similar to br_target(), call to get all implicitly ended blocks by a return
    pub fn return_target(&self) -> BranchTarget {
        BranchTarget {
            absolute_instr: if let Some(Function { end }) = self.block_stack.first() {
                *end
            } else {
                panic!("missing function block on block stack")
            },
            ended_blocks: self.block_stack.iter().rev().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BranchTarget {
    /// the resolved absolute instruction index (from the relative branch label)
    /// NOTE is either a begin or end, need to add +1 to get the next "real" instruction
    pub absolute_instr: Idx<Instr>,
    /// all blocks that are implicitly ended when performing the branch (including the target block)
    /// in the order how they are left (i.e., innermost [== current block] to outermost [== target block])
    pub ended_blocks: Vec<BlockStackElement>,
}
