// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I59 — Type resolver (+ field offsets)

//! Field layout calculator — computes byte offsets for struct/enum fields.
//!
//! [`calculate_positions`] takes a list of `(name, size, alignment)` fields
//! and assigns each a byte offset within a record, minimising gaps via a
//! `BTreeMap`-based gap tracker.  Fields are sorted largest-first to reduce
//! padding.  Called by `typedef.rs` during type resolution.

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
        // B2-runtime (2026-04-13): a unit variant in a mixed struct-enum
        // has only the enum discriminant field.  Without this, `size`
        // stays 0 and `Store::claim(0)` panics "Incomplete record".
        // Ensure at least 1 byte (the discriminant) is accounted for.
        *size = 1;
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

/// Group-aware variant of [`calculate_positions`].  Treats every
/// linked-field group as a SINGLE atomic unit during packing — the
/// layout routine reserves the group's `total_size` bytes at a
/// `group_alignment`-aligned position, then expands each member to
/// its in-group offset.  Non-group fields use the same gap-tracker
/// as `calculate_positions`.
///
/// **Why**: the index bookkeeping triple `[#left_N, #right_N,
/// #color_N]` and tuple element fields `[_0, _1, …, _N]` must stay
/// CONTIGUOUS so consumers (`tree::add` for index, tuple-as-arg
/// inflation for tuples) can use simple offset arithmetic.  Without
/// group-awareness, `calculate_positions` packs fields individually
/// (largest-first by alignment) and a 1-byte `bool` member of an
/// index group can be pulled to the trailing 1-byte fill region of
/// the host struct — breaking `tree::add`'s `color = left + 8`
/// expectation.
///
/// **Args**:
/// - `fields[i] = (size, alignment)` per host field, in original
///   declaration order (matches what `calculate_positions` accepts).
/// - `groups[g] = (member_field_indices, group_total_size,
///   group_alignment, member_in_group_offsets)` — one entry per
///   linked-field group.  `member_field_indices[k]` is the index
///   into `fields` of the k-th member; `member_in_group_offsets[k]`
///   is its byte offset within the atomic group block.
/// - `sub` / `size` / `alignment` — same role as `calculate_positions`.
///
/// **Returns**: positions for every field in `fields`, with group
/// members at `group_anchor + member_in_group_offsets[k]`.
///
/// Panics if a field index appears in two different groups (groups
/// must be disjoint) or if a member offset extends past the
/// declared `group_total_size`.
pub fn calculate_positions_with_groups(
    fields: &[(u16, u8)],
    groups: &[(Vec<u16>, u16, u8, Vec<u16>)],
    sub: bool,
    size: &mut u16,
    alignment: &mut u8,
) -> Vec<u16> {
    // Validate group disjointness + offset bounds; panic-on-bug
    // since this is a layout invariant the caller is responsible
    // for upholding.
    let mut field_to_group: BTreeMap<u16, usize> = BTreeMap::new();
    for (g_nr, (members, total_size, _g_align, offsets)) in groups.iter().enumerate() {
        assert_eq!(
            members.len(),
            offsets.len(),
            "group {g_nr}: member count {} != offsets count {}",
            members.len(),
            offsets.len(),
        );
        for (k, &m_idx) in members.iter().enumerate() {
            assert!(
                offsets[k] + fields[m_idx as usize].0 <= *total_size,
                "group {g_nr} member {k} (size {}) at offset {} exceeds total_size {}",
                fields[m_idx as usize].0,
                offsets[k],
                total_size,
            );
            assert!(
                field_to_group.insert(m_idx, g_nr).is_none(),
                "field {m_idx} appears in multiple groups",
            );
        }
    }

    // Build the virtual field list: each non-group field stays as-is,
    // each group becomes ONE virtual field at the position of its
    // first member in the original order.  Skip subsequent members
    // (they're handled by the group's offset expansion).
    //
    // Padding rule: `calculate_positions` advances `pos += field_size`
    // and assumes ABI-style fields where size is a multiple of
    // alignment.  A group with size that's not a multiple of its
    // alignment (e.g. index triple {int4, int4, bool1} → size 9,
    // alignment 4) breaks that invariant — leaving `pos` misaligned
    // for the next field in the same alignment iteration.
    //
    // We pad the group's virtual size up to a multiple of its
    // alignment ONLY when there is a subsequent virtual field with
    // alignment ≥ the group's alignment.  Trailing groups (and groups
    // followed only by smaller-alignment fields, which are placed in
    // later iterations and don't need the same alignment) skip
    // padding so the struct's total size doesn't bloat unnecessarily.
    //
    // First pass: build the unpadded virtual list and remember per-
    // entry alignment so the second pass can decide whether to pad.
    let mut virtual_fields: Vec<(u16, u8)> = Vec::with_capacity(fields.len());
    let mut virtual_origin: Vec<VirtualOrigin> = Vec::with_capacity(fields.len());
    let mut group_seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (f_idx, &(f_size, f_align)) in fields.iter().enumerate() {
        if let Some(&g_nr) = field_to_group.get(&(f_idx as u16)) {
            if group_seen.insert(g_nr) {
                let (_, total_size, g_align, _) = &groups[g_nr];
                virtual_fields.push((*total_size, *g_align));
                virtual_origin.push(VirtualOrigin::Group(g_nr));
            }
            // Skip non-first members (already covered by virtual entry).
        } else {
            virtual_fields.push((f_size, f_align));
            virtual_origin.push(VirtualOrigin::Single(f_idx));
        }
    }
    // Second pass: pad groups that have a later same-or-greater
    // alignment field.  Without padding, that follow-on field would
    // be placed at a misaligned `pos` since `pos += field_size`
    // doesn't realign on its own.
    for v_idx in 0..virtual_fields.len() {
        if !matches!(virtual_origin[v_idx], VirtualOrigin::Group(_)) {
            continue;
        }
        let (cur_size, cur_align) = virtual_fields[v_idx];
        let cur_align_u16 = u16::from(cur_align.max(1));
        let rem = cur_size % cur_align_u16;
        if rem == 0 {
            continue; // already aligned, no padding needed
        }
        let needs_padding = virtual_fields[v_idx + 1..]
            .iter()
            .any(|&(_, a)| a >= cur_align);
        if needs_padding {
            virtual_fields[v_idx].0 = cur_size + (cur_align_u16 - rem);
        }
    }

    // Run the standard packer on the virtual list.
    let virtual_positions = calculate_positions(&virtual_fields, sub, size, alignment);

    // Expand virtual positions back to per-field positions.
    let mut positions = vec![0u16; fields.len()];
    for (v_idx, &v_pos) in virtual_positions.iter().enumerate() {
        match &virtual_origin[v_idx] {
            VirtualOrigin::Single(f_idx) => {
                positions[*f_idx] = v_pos;
            }
            VirtualOrigin::Group(g_nr) => {
                let (members, _, _, offsets) = &groups[*g_nr];
                for (k, &m_idx) in members.iter().enumerate() {
                    positions[m_idx as usize] = v_pos + offsets[k];
                }
            }
        }
    }
    positions
}

enum VirtualOrigin {
    /// Standalone field — virtual position is its position.
    Single(usize),
    /// Group anchor — virtual position is the group's start; expand
    /// to per-member positions via the group's offsets.
    Group(usize),
}
