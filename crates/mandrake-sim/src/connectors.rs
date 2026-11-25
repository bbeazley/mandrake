use crate::engine_rng::{EngineWorld, update_monster_impassability_for_cell};
use crate::passability::connector_cells;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectorPost {
    pub linked: bool,
    pub scenery: Option<usize>,
    pub position: (i32, i32),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Connector {
    pub posts: Vec<ConnectorPost>,
}

impl Connector {
    pub fn new(posts: &[(Option<usize>, (i32, i32))]) -> Self {
        let n = posts.len();
        let posts = (0..n)
            .map(|k| ConnectorPost {
                linked: k + 1 < n && posts[k].0.is_some() && posts[k + 1].0.is_some(),
                scenery: posts[k].0,
                position: posts[k].1,
            })
            .collect();
        Self { posts }
    }
}

impl EngineWorld {
    fn connector_post(&self, post: &ConnectorPost) -> Option<(i32, i32)> {
        let s = self.scenery.get(post.scenery?)?;
        (!s.deleted).then_some(s.position)
    }

    pub fn connectors_changing(&self) -> bool {
        self.connectors
            .iter()
            .flat_map(|c| &c.posts)
            .any(|p| self.connector_post(p).is_none())
    }

    pub fn update_connectors(&mut self) {
        for c in 0..self.connectors.len() {
            let mut k = self.connectors[c].posts.len();
            while k > 0 {
                k -= 1;
                let post = self.connectors[c].posts[k];
                if let Some(p) = self.connector_post(&post) {
                    self.connectors[c].posts[k].position = p;
                    continue;
                }
                let n = self.connectors[c].posts.len();
                let prev = if k == 0 { n - 1 } else { k - 1 };
                let next = if k == n - 1 { 0 } else { k + 1 };
                if self.connectors[c].posts[prev].linked {
                    self.connectors[c].posts[prev].linked = false;
                    let a = self.connectors[c].posts[prev].position;
                    self.unkill_connector_line(a, post.position);
                }
                if self.connectors[c].posts[k].linked {
                    self.connectors[c].posts[k].linked = false;
                    let b = self.connectors[c].posts[next].position;
                    self.unkill_connector_line(post.position, b);
                }
                self.connectors[c].posts.remove(k);
            }
        }
    }

    fn unkill_connector_line(&mut self, a: (i32, i32), b: (i32, i32)) {
        for (x, y) in connector_cells(a, b) {
            self.cells.set_connector_cell(x, y, false);
            update_monster_impassability_for_cell(self, x, y);
        }
        self.cells_changed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_neighbouring_posts_that_both_exist_link() {
        let c = Connector::new(&[
            (Some(0), (0, 0)),
            (Some(1), (512, 0)),
            (None, (0, 0)),
            (Some(2), (1024, 0)),
            (Some(3), (1536, 0)),
        ]);
        let links: Vec<bool> = c.posts.iter().map(|p| p.linked).collect();
        assert_eq!(links, [true, false, false, true, false]);
    }
}
