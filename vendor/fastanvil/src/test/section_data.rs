use crate::{min_bits_for_n_states, StatesIter};

#[test]
fn packed_lookup_keeps_palette_boundaries_and_padding() {
    use crate::BlockData;
    use mcmap_nbt::{nbt, LongArray, Value};
    for count in [1usize, 2, 16, 17, 32, 33, 65, 129, 257, 513, 1025, 2049] {
        let bits = min_bits_for_n_states(count).max(4);
        let per_long = 64 / bits;
        let mut words = vec![0i64; (4096 + per_long - 1) / per_long];
        for index in 0..4096 {
            words[index / per_long] |=
                (((index % count) as u64) << ((index % per_long) * bits)) as i64;
        }
        let palette = Value::List((0..count).map(|index| Value::Int(index as i32)).collect());
        let bytes =
            mcmap_nbt::to_bytes(&nbt!({ "palette": palette, "data": LongArray::new(words) }))
                .unwrap();
        let data: BlockData<usize> = mcmap_nbt::from_bytes(&bytes).unwrap();
        for index in 0..4096 {
            assert_eq!(
                data.at(index & 15, index >> 8, (index >> 4) & 15),
                Some(&(index % count))
            );
        }
    }
}

#[test]
fn packed_singletons_and_missing_data_keep_air_fallbacks() {
    use crate::{BiomeData, BlockData};
    use mcmap_nbt::nbt;
    let single: BlockData<usize> =
        mcmap_nbt::from_bytes(&mcmap_nbt::to_bytes(&nbt!({"palette": [7]})).unwrap()).unwrap();
    let missing: BlockData<usize> =
        mcmap_nbt::from_bytes(&mcmap_nbt::to_bytes(&nbt!({"palette": [7, 9]})).unwrap()).unwrap();
    assert_eq!(single.at(15, 15, 15), Some(&7));
    assert_eq!(missing.at(15, 15, 15), None);
    assert_eq!(BlockData::<usize>::default().at(0, 0, 0), None);
    assert_eq!(BiomeData::<usize>::default().at(0, 0, 0), None);
}

#[test]
fn iter_zeroes() {
    let actual: Vec<_> = StatesIter::new(4, 16, &[0]).collect();
    let expected = [0; 16];
    assert_eq!(expected[..], actual);
}

#[test]
fn iter_one() {
    let actual: Vec<_> = StatesIter::new(4, 16, &[1]).collect();
    let expected = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(expected[..], actual);
}

#[test]
fn iter_capped() {
    let actual: Vec<_> = StatesIter::new(4, 2, &[1]).collect();
    let expected = [1, 0];
    assert_eq!(expected[..], actual);
}

#[test]
fn iter_skips_padding() {
    let actual: Vec<_> = StatesIter::new(60, 3, &[1, 2, 3]).collect();
    let expected = [1, 2, 3];
    assert_eq!(expected[..], actual);
}

#[test]
fn min_bits() {
    let ideal = |n: usize| (n as f64).log2().ceil() as usize;

    assert_eq!(0, min_bits_for_n_states(1));
    assert_eq!(1, min_bits_for_n_states(2));
    assert_eq!(2, min_bits_for_n_states(3));
    assert_eq!(2, min_bits_for_n_states(4));
    assert_eq!(3, min_bits_for_n_states(5));
    assert_eq!(3, min_bits_for_n_states(5));

    for i in 1..16 * 16 * 16 {
        assert_eq!(ideal(i), min_bits_for_n_states(i));
    }
}
