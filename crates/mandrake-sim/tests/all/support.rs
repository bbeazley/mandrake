use mandrake_common::FourCC;
use mandrake_tag::MapActionParameter;

#[path = "../../../mandrake-common/tests/support/require_data.rs"]
pub mod data;

pub fn param(name: &[u8; 4], kind: i16, values: &[i32]) -> MapActionParameter {
    let per = MapActionParameter::scalars_per_element(kind).max(1);
    MapActionParameter {
        kind,
        count: (values.len() / per) as i16,
        name: FourCC::new(*name),
        values: values.to_vec(),
    }
}
