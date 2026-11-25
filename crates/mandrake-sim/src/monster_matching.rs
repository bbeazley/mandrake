struct Assignment {
    row_to_col: Vec<u8>,
    col_to_row: Vec<u8>,
}

fn auction(
    cost: &[Vec<u64>],
    rows: usize,
    cols: usize,
    row_map: Option<&[u8]>,
    max_cost: u64,
) -> Option<Assignment> {
    let size = rows.max(cols);
    let mut prices = vec![0u64; size];
    let mut epsilon = max_cost;
    loop {
        let mut row_to_col = vec![0xffu8; size];
        let mut col_to_row = vec![0xffu8; size];
        let mut open: Vec<u8> = (0..size).map(|r| r as u8).collect();
        let mut bids = 0usize;
        while let Some(&row) = open.last() {
            let r = usize::from(row);
            let mut best = 0usize;
            let mut low = u64::MAX;
            for (j, &price) in prices.iter().enumerate() {
                let total = if r < rows && j < cols {
                    let source = row_map.map_or(r, |m| usize::from(m[r]));
                    price.wrapping_add(cost[source][j])
                } else {
                    price
                };
                if total < low {
                    low = total;
                    best = j;
                }
            }
            let previous = col_to_row[best];
            if previous == 0xff {
                open.pop();
            } else {
                row_to_col[usize::from(previous)] = 0xff;
                *open.last_mut().expect("a row") = previous;
            }
            col_to_row[best] = row;
            bids += 1;
            prices[best] = prices[best].wrapping_add(epsilon);
            row_to_col[r] = best as u8;
            if bids > size * 400 {
                return None;
            }
        }
        if epsilon == 1 {
            return Some(Assignment {
                row_to_col,
                col_to_row,
            });
        }
        epsilon = if epsilon > 7 { epsilon >> 2 } else { 1 };
    }
}

fn distance2(a: (i32, i32), b: (i32, i32)) -> u64 {
    let (dx, dy) = (
        i64::from(a.0) - i64::from(b.0),
        i64::from(a.1) - i64::from(b.1),
    );
    (dx.wrapping_mul(dx) as u64).wrapping_add(dy.wrapping_mul(dy) as u64)
}

pub fn match_with_destinations(from: &[(i32, i32)], to: &[(i32, i32)]) -> Option<Vec<usize>> {
    let n = from.len();
    let mut max = 0u64;
    let cost: Vec<Vec<u64>> = from
        .iter()
        .map(|&a| {
            to[..n]
                .iter()
                .map(|&b| {
                    let d = distance2(a, b);
                    max = max.max(d);
                    d.wrapping_mul(n as u64)
                })
                .collect()
        })
        .collect();
    let a = auction(&cost, n, n, None, max)?;
    Some(a.col_to_row.iter().map(|&r| usize::from(r)).collect())
}

pub fn match_with_targets(attackers: &[(i32, i32)], targets: &[(i32, i32)]) -> Option<Vec<usize>> {
    let (n, m) = (attackers.len(), targets.len());
    let size = n.max(m) as u64;
    let mut max = 0u64;
    let cost: Vec<Vec<u64>> = attackers
        .iter()
        .map(|&a| {
            targets
                .iter()
                .map(|&b| {
                    let d = distance2(a, b);
                    max = max.max(d);
                    d.wrapping_mul(size)
                })
                .collect()
        })
        .collect();
    let mut result = vec![0xffu8; n];
    let mut rows: Vec<u8> = (0..n).map(|r| r as u8).collect();
    while !rows.is_empty() {
        let a = auction(&cost, rows.len(), m, Some(&rows), max)?;
        let mut left = Vec::new();
        for (k, &row) in rows.iter().enumerate() {
            let col = a.row_to_col[k];
            if usize::from(col) < m {
                result[usize::from(row)] = col;
            } else {
                left.push(row);
            }
        }
        rows = left;
    }
    Some(result.iter().map(|&c| usize::from(c)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_monster_takes_the_near_destination() {
        let from = [(0, 0), (1000, 0), (2000, 0)];
        let to = [(2100, 0), (100, 0), (1100, 0)];
        assert_eq!(match_with_destinations(&from, &to), Some(vec![2, 0, 1]));
    }

    #[test]
    fn more_attackers_than_targets_share_them_out() {
        let attackers = [(0, 0), (10, 0), (1000, 0), (1010, 0)];
        let targets = [(0, 100), (1000, 100)];
        assert_eq!(
            match_with_targets(&attackers, &targets),
            Some(vec![0, 0, 1, 1])
        );
    }

    #[test]
    fn more_targets_than_attackers_take_the_nearest() {
        let attackers = [(1000, 0), (0, 0)];
        let targets = [(0, 100), (500, 100), (1000, 100)];
        assert_eq!(match_with_targets(&attackers, &targets), Some(vec![2, 0]));
    }
}
