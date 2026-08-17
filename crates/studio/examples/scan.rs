//! Print what the history rail will show, for a real library on disk.
//! Useful for eyeballing storage without launching the GUI.
//!
//! ```sh
//! cargo run -p studio --example scan
//! ```

fn main() {
    let lib = studio::Library::new(studio::Library::default_root());

    match lib.scan() {
        Ok(takes) => {
            println!("{} takes in {}\n", takes.len(), lib.root().display());
            for t in takes {
                println!(
                    "  {:<18} take {:>2}   {:>6.1}s   {}   {}",
                    t.title,
                    t.take,
                    t.duration_secs,
                    "★".repeat(t.rating as usize) + &"☆".repeat(5 - t.rating as usize),
                    t.caption
                );
            }
        }
        Err(e) => println!("scan failed: {e}"),
    }
}
