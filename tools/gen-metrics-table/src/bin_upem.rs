fn main() {
    for (name, data) in [("Regular", dejavu::sans::regular()), ("Bold", dejavu::sans::bold())] {
        let f = ttf_parser::Face::parse(data, 0).unwrap();
        println!("{name}: units_per_em={:?} ascender={} descender={}", f.units_per_em(), f.ascender(), f.descender());
    }
}
