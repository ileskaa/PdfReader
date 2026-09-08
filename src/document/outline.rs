#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub depth: usize,
    pub page: Option<usize>,
}

pub fn flatten(nodes: &[mupdf::Outline]) -> Vec<OutlineItem> {
    let mut out = Vec::new();
    push_level(nodes, 0, &mut out);
    out
}

fn push_level(nodes: &[mupdf::Outline], depth: usize, out: &mut Vec<OutlineItem>) {
    for node in nodes {
        out.push(OutlineItem {
            title: node.title.clone(),
            depth,
            page: node.dest.map(|d| d.loc.page_number as usize),
        });
        push_level(&node.down, depth + 1, out);
    }
}
