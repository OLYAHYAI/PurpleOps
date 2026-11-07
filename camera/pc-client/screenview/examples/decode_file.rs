// Vérifie le découpage NAL + décodage OpenH264 sur un fichier Annex-B, lu par morceaux.
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let data = std::fs::read(&path).unwrap();
    let mut dec = Decoder::new().unwrap();
    let (mut frames, mut errs, mut dims) = (0, 0, (0, 0));
    let mut pending = Vec::new();
    let t0 = std::time::Instant::now();
    for chunk in data.chunks(1500) {
        pending.extend_from_slice(chunk);
        let s = starts(&pending);
        if s.len() >= 2 {
            for w in s.windows(2) {
                match dec.decode(&pending[w[0]..w[1]]) { Ok(Some(y)) => { frames += 1; dims = y.dimensions(); } Ok(None) => {}, Err(_) => errs += 1 }
            }
            let l = *s.last().unwrap(); pending.drain(..l);
        }
    }
    if let Some(&f) = starts(&pending).first() { if let Ok(Some(_)) = dec.decode(&pending[f..]) { frames += 1; } }
    println!("{path}: {frames} images {dims:?}, {errs} erreurs, {:.1} ms/img", t0.elapsed().as_secs_f64()*1000.0/frames.max(1) as f64);
}
fn starts(b: &[u8]) -> Vec<usize> { let mut v=vec![]; let mut i=0; while i+3<=b.len(){ if b[i]==0&&b[i+1]==0&&b[i+2]==1 { v.push(if i>0&&b[i-1]==0 {i-1} else {i}); i+=3 } else { i+=1 } } v }
