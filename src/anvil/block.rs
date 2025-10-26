// Block representation

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub name: &'static str,
    pub archetype: BlockArchetype,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockArchetype {
    Airy,      // Air blocks
    Watery,    // Water blocks
    Solid,     // Normal solid blocks
}

impl Block {
    pub const fn new(name: &'static str, archetype: BlockArchetype) -> Self {
        Self { name, archetype }
    }
}

// Common blocks
pub const AIR: Block = Block::new("minecraft:air", BlockArchetype::Airy);
pub const WATER: Block = Block::new("minecraft:water", BlockArchetype::Watery);
pub const STONE: Block = Block::new("minecraft:stone", BlockArchetype::Solid);
pub const GRASS: Block = Block::new("minecraft:grass_block", BlockArchetype::Solid);
