use mandrake_sim::legion_action::closest_point_on_line_segment_from_point;

#[test]
fn the_closest_point() {
    let f = closest_point_on_line_segment_from_point;
    let (a, b) = ((0, 0), (1024, 0));
    assert_eq!(f((512, 100), a, b), ((512, 0), (512, 0)));
    assert_eq!(f((-512, 0), a, b), ((-512, 0), a));
    assert_eq!(f((2048, 0), a, b), ((2048, 0), b));
    assert_eq!(f((5, 5), (0, 0), (10, 0)), ((10, 0), (10, 0)));
}
