use fixture_worker::bake_route;

fn main() {
    let route = std::env::args()
        .skip_while(|arg| arg != "--route")
        .nth(1)
        .unwrap_or_else(|| "examples/route.gpx".to_owned());
    println!("{}", bake_route(&route));
}

