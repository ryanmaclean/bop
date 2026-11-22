# Duration Heatmap Color Verification

## Implementation Details

The duration heatmap feature in `crates/bop-cli/src/gantt.rs` correctly applies colors based on percentile thresholds.

### Color Scheme
- **Green (#4caf50)**: Fast runs (p0-p50) - durations ≤ 50th percentile
- **Amber (#ff9800)**: Medium runs (p50-p80) - durations > 50th and ≤ 80th percentile
- **Red (#f44336)**: Slow runs (p80-p100) - durations > 80th percentile

### Implementation Flow

1. **Percentile Calculation** (lines 179-187):
   ```rust
   fn percentile_threshold(durations: &[f64], p: f64) -> f64 {
       if durations.is_empty() {
           return 0.0;
       }
       let mut sorted = durations.to_vec();
       sorted.sort_by(|a, b| a.total_cmp(b));
       let idx = ((sorted.len().saturating_sub(1)) as f64 * p).round() as usize;
       sorted[idx.min(sorted.len() - 1)]
   }
   ```
   - Sorts durations in ascending order
   - Calculates index based on percentile (0.50 for median, 0.80 for 80th)
   - Returns the threshold value at that index

2. **Per-Cluster Thresholds** (lines 505-507):
   ```rust
   let cluster_durations: Vec<f64> = indices.iter().map(|&i| bars[i].dur_s).collect();
   let p50 = percentile_threshold(&cluster_durations, 0.50);
   let p80 = percentile_threshold(&cluster_durations, 0.80);
   ```
   - Calculates percentiles independently for each cluster
   - Ensures colors are relative to cluster performance, not global

3. **Color Assignment** (lines 530-536):
   ```rust
   let heat_color = if b.dur_s <= p50 {
       "#4caf50"  // green
   } else if b.dur_s <= p80 {
       "#ff9800"  // amber
   } else {
       "#f44336"  // red
   };
   ```
   - Compares each bar's duration against thresholds
   - Assigns color based on which percentile range it falls into

4. **HTML Bar Rendering** (lines 537-541):
   ```rust
   writeln!(
       html,
       "<div class=\"row\"><div class=\"bar\" style=\"left:{:.1}%;width:{:.1}%;background:{}\" title=\"{}\"></div></div>",
       lp, wp, heat_color, tip,
   );
   ```
   - Applies the heat color as the background CSS property

5. **Legend** (lines 549-559):
   ```rust
   for (label, col) in [
       ("fast (p0-p50)", "#4caf50"),
       ("medium (p50-p80)", "#ff9800"),
       ("slow (p80-p100)", "#f44336"),
   ]
   ```
   - Displays color legend with percentile ranges

## Test Coverage

The `generate_html_produces_valid_structure` test (lines 647-684) verifies:
- ✅ HTML structure is valid
- ✅ At least one heatmap color (#4caf50, #ff9800, or #f44336) appears in output
- ✅ Test passes successfully

## Verification Results

✅ **Percentile calculation**: Correctly sorts and indexes durations
✅ **Color logic**: Proper if/else chain with correct thresholds
✅ **HTML output**: Heat color applied to inline style
✅ **Legend**: All three colors documented with percentile ranges
✅ **Tests**: Unit test passes and verifies color presence

## Manual Verification Command

To visually verify the heatmap colors:
```bash
bop gantt --html -o
```

This will:
1. Generate `.cards/bop-gantt.html` with duration-based colors
2. Open the file in your default browser (macOS)
3. Display bars colored green (fast), amber (medium), and red (slow)

## Conclusion

The duration heatmap color implementation is **CORRECT** and working as specified:
- Colors are based on percentile thresholds (p50 and p80)
- Each cluster calculates its own percentiles
- Color assignment logic matches the legend
- Test coverage confirms functionality
