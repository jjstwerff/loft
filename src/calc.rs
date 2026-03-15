// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use std::cmp::Ordering;
use std::collections::BTreeMap;

/**
    vector: do not reserve 4 bytes for the record length.
    sub: part of a sub, reserve the first byte for the record type.
*/
pub fn calculate_positions(
    fields: &[(u16, u8)],
    sub: bool,
    size: &mut u16,
    alignment: &mut u8,
) -> Vec<u16> {
    // A gap on position with size. The only gaps allowed are due to their alignments
    let mut gaps = BTreeMap::new();
    // Calculated position for each field on number.
    let mut positions = BTreeMap::new();
    let mut pos = 0;
    // Keep space for the type for an EnumValue.
    if sub {
        // Start on the first 8 byte alignment position.
        pos = 8;
        positions.insert(0, 0);
        gaps.insert(1, 7);
    }
    for al in [8, 4, 2, 1] {
        for (nr, (field_size, align)) in fields.iter().enumerate() {
            if sub && nr == 0 {
                continue;
            }
            if *align == al {
                if al > *alignment {
                    *alignment = al;
                }
                let mut first = 0;
                let mut first_size = 0;
                for (&gap_pos, &size) in &gaps {
                    if size >= *field_size {
                        first = gap_pos;
                        first_size = size;
                        break;
                    }
                }
                match first_size.cmp(field_size) {
                    Ordering::Equal => {
                        gaps.remove(&first);
                        positions.insert(nr, first);
                        if *size < first + field_size {
                            *size = first + field_size;
                        }
                    }
                    Ordering::Greater => {
                        // claim the back side of the gap
                        let new_size = first_size - field_size;
                        gaps.insert(first, new_size);
                        positions.insert(nr, first + new_size);
                        if *size < first + new_size + field_size {
                            *size = first + new_size + field_size;
                        }
                    }
                    Ordering::Less => {
                        positions.insert(nr, pos);
                        pos += field_size;
                        *size = pos;
                    }
                }
            }
        }
    }
    let mut result = Vec::new();
    for (_, pos) in positions {
        result.push(pos);
    }
    result
}
