/// Build an NBT value; numeric suffixes select the on-disk numeric tag.
#[macro_export]
macro_rules! nbt {
    (@list [$($values:expr,)*]) => { vec![$($values,)*] };
    (@list [$($values:expr,)*] , $($rest:tt)*) => { $crate::nbt!(@list [$($values,)*] $($rest)*) };
    (@list [$($values:expr,)*] {$($v:tt)*} $($rest:tt)*) => { $crate::nbt!(@list [$($values,)* $crate::nbt!({$($v)*}),] $($rest)*) };
    (@list [$($values:expr,)*] [$($v:tt)*] $($rest:tt)*) => { $crate::nbt!(@list [$($values,)* $crate::nbt!([$($v)*]),] $($rest)*) };
    (@list [$($values:expr,)*] $v:expr, $($rest:tt)*) => { $crate::nbt!(@list [$($values,)* $crate::Value::from($v),] $($rest)*) };
    (@list [$($values:expr,)*] $v:expr) => { vec![$($values,)* $crate::Value::from($v)] };
    (@map $map:ident) => {};
    (@map $map:ident , $($rest:tt)*) => { $crate::nbt!(@map $map $($rest)*); };
    (@map $map:ident $key:tt : {$($v:tt)*} $($rest:tt)*) => {
        $map.insert(String::from($key), $crate::nbt!({$($v)*})); $crate::nbt!(@map $map $($rest)*);
    };
    (@map $map:ident $key:tt : [$($v:tt)*] $($rest:tt)*) => {
        $map.insert(String::from($key), $crate::nbt!([$($v)*])); $crate::nbt!(@map $map $($rest)*);
    };
    (@map $map:ident $key:tt : $v:expr, $($rest:tt)*) => {
        $map.insert(String::from($key), $crate::Value::from($v)); $crate::nbt!(@map $map $($rest)*);
    };
    (@map $map:ident $key:tt : $v:expr) => { $map.insert(String::from($key), $crate::Value::from($v)); };
    ([B; $($v:expr),* $(,)?]) => { $crate::Value::ByteArray($crate::ByteArray::new(vec![$($v),*])) };
    ([I; $($v:expr),* $(,)?]) => { $crate::Value::IntArray($crate::IntArray::new(vec![$($v),*])) };
    ([L; $($v:expr),* $(,)?]) => { $crate::Value::LongArray($crate::LongArray::new(vec![$($v),*])) };
    ([$($v:tt)*]) => { $crate::Value::List($crate::nbt!(@list [] $($v)*)) };
    ({$($v:tt)*}) => {{
        let mut map = ::std::collections::HashMap::new();
        $crate::nbt!(@map map $($v)*);
        $crate::Value::Compound(map)
    }};
    ($v:expr) => { $crate::Value::from($v) };
}
