// Index table facets, screenshot rotators, studied-design strips. Everything
// is one delegated click handler; no framework, no bundle.
document.addEventListener('click', function (e) {
  var fb = e.target.closest('.facet-btn');
  if (fb) {
    document.querySelectorAll('.facet-btn[data-facet="' + fb.dataset.facet + '"]')
      .forEach(function (b) { b.classList.remove('active'); });
    fb.classList.add('active');
    var active = {};
    document.querySelectorAll('.facet-btn.active').forEach(function (b) {
      if (b.dataset.value) active[b.dataset.facet] = b.dataset.value;
    });
    document.querySelectorAll('.run-table tbody tr').forEach(function (row) {
      var show = true;
      if (active.mode && row.dataset.mode !== active.mode) show = false;
      if (active.coaster && row.dataset.coaster !== active.coaster) show = false;
      if (active.harness && row.dataset.harness !== active.harness) show = false;
      if (active.model && row.dataset.model !== active.model) show = false;
      row.style.display = show ? '' : 'none';
    });
    return;
  }

  // Column sorting. The page ships sorted by score, descending; clicking a
  // header re-sorts, clicking the active one flips direction.
  var th = e.target.closest('.run-table th.sortable');
  if (th) {
    var key = th.dataset.sort;
    var table = th.closest('table');
    var wasDesc = th.classList.contains('sorted-desc');
    var wasAsc = th.classList.contains('sorted-asc');
    // First click on a column uses its natural direction (scores high-first,
    // names A-first); clicking the active column flips it.
    var dir = wasDesc ? 'asc' : wasAsc ? 'desc' : (th.dataset.dir || 'desc');
    table.querySelectorAll('th').forEach(function (h) {
      h.classList.remove('sorted-asc', 'sorted-desc');
    });
    th.classList.add(dir === 'asc' ? 'sorted-asc' : 'sorted-desc');

    var body = table.querySelector('tbody');
    var rows = Array.prototype.slice.call(body.rows);
    var sign = dir === 'asc' ? 1 : -1;
    rows.sort(function (a, b) {
      var x = a.dataset[key], y = b.dataset[key];
      var nx = parseFloat(x), ny = parseFloat(y);
      if (!isNaN(nx) && !isNaN(ny)) return sign * (nx - ny);
      return sign * String(x).localeCompare(String(y));
    });
    rows.forEach(function (row) { body.appendChild(row); });
    return;
  }

  var strip = e.target.closest('.strip-btn');
  if (strip) {
    var track = strip.parentElement.querySelector('.gallery');
    var card = track.querySelector('figure');
    var step = (card ? card.offsetWidth + 10 : 195) * 2;
    track.scrollBy({ left: strip.classList.contains('strip-next') ? step : -step, behavior: 'smooth' });
    return;
  }

  var btn = e.target.closest('.rot-btn');
  if (!btn) return;
  var rot = btn.closest('.rotator');
  var shots = JSON.parse(rot.dataset.shots);
  var step = btn.classList.contains('rot-next') ? 1 : shots.length - 1;
  var i = (parseInt(rot.dataset.i, 10) + step) % shots.length;
  var labels = JSON.parse(rot.dataset.labels);
  rot.dataset.i = i;
  rot.querySelector('img').src = shots[i];
  rot.querySelector('.rot-count').textContent = labels[i];
});

// Hide the strip arrows when everything already fits.
function markScrollableStrips() {
  document.querySelectorAll('.strip').forEach(function (strip) {
    var track = strip.querySelector('.gallery');
    strip.dataset.scrollable = track.scrollWidth > track.clientWidth + 4 ? 'true' : 'false';
  });
}
window.addEventListener('resize', markScrollableStrips);
window.addEventListener('load', markScrollableStrips);
markScrollableStrips();

// Trace page: show everything, only tool calls, or only rejected calls.
document.querySelectorAll('[data-trace-filter]').forEach(function (btn) {
  btn.addEventListener('click', function () {
    var want = btn.dataset.traceFilter;
    document.querySelectorAll('[data-trace-filter]').forEach(function (b) {
      b.classList.toggle('active', b === btn);
    });
    document.querySelectorAll('.trace-row').forEach(function (row) {
      var show = want === ''
        || (want === 'tool' && row.dataset.kind === 'tool')
        || (want === 'failed' && row.dataset.failed === 'true');
      row.hidden = !show;
    });
  });
});

// Head-to-head: pick two finished model runs and compare them client-side.
// All contenders are embedded as JSON, so any pairing renders without a fetch,
// and the pick is reflected in ?a=&b= for shareable matchups.
(function () {
  var root = document.getElementById('compare');
  if (!root) return;
  var data = JSON.parse(document.getElementById('contenders').textContent);
  var byId = {};
  data.forEach(function (c) { byId[c.id] = c; });
  var selects = { a: root.querySelector('[data-side="a"]'), b: root.querySelector('[data-side="b"]') };
  var result = document.getElementById('vs-result');

  // Group options by coaster so the dropdown reads scenario by scenario.
  function fillSelect(sel) {
    var groups = {};
    data.forEach(function (c) { (groups[c.coaster] = groups[c.coaster] || []).push(c); });
    Object.keys(groups).sort().forEach(function (coaster) {
      var og = document.createElement('optgroup');
      og.label = coaster;
      groups[coaster].forEach(function (c) {
        var o = document.createElement('option');
        o.value = c.id;
        var score = c.score == null ? 'no score' : c.score.toFixed(2);
        o.textContent = c.model + ' · ' + c.run + ' (' + score + ')';
        og.appendChild(o);
      });
      sel.appendChild(og);
    });
  }
  fillSelect(selects.a);
  fillSelect(selects.b);

  var num = function (n) { return n == null ? '—' : (+n).toFixed(2); };
  var intFmt = function (n) { return n == null ? '—' : String(n); };
  var tokens = function (n) {
    if (!n) return '—';
    if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
    if (n >= 1e3) return Math.round(n / 1e3) + 'k';
    return String(Math.round(n));
  };
  var money = function (n) { return n == null ? '—' : '$' + (+n).toFixed(2); };

  // Metrics shown as two-sided bars. dir: 'up' higher wins, 'down' lower wins,
  // null neutral (shown, no winner). fmt formats the printed value.
  var METRICS = [
    { key: 'score', label: 'excitement', dir: 'up', fmt: num, headline: true },
    { key: 'intensity', label: 'intensity', dir: null, fmt: num },
    { key: 'nausea', label: 'nausea', dir: null, fmt: num },
    { key: 'similarity', label: 'similarity to stock', dir: 'down', fmt: num },
    { key: 'airtime', label: 'airtime', dir: 'up', fmt: intFmt },
    { key: 'drops', label: 'drops', dir: 'up', fmt: intFmt },
    { key: 'tokens', label: 'tokens', dir: 'down', fmt: tokens },
    { key: 'cost', label: 'cost', dir: 'down', fmt: money }
  ];

  // Which side wins a metric, or '' for tie/neutral/unknown.
  function winner(m, a, b) {
    if (m.dir == null) return '';
    var va = a[m.key], vb = b[m.key];
    if (va == null && vb == null) return '';
    if (va == null) return 'b';
    if (vb == null) return 'a';
    if (va === vb) return '';
    var aWins = m.dir === 'up' ? va > vb : va < vb;
    return aWins ? 'a' : 'b';
  }

  function card(c, side) {
    var thumb = c.thumb
      ? '<a href="' + c.href + '"><img src="' + c.thumb + '" alt="' + c.model + '"></a>'
      : '<div class="no-preview">no image</div>';
    return '<div class="vs-card vs-' + side + '">' + thumb
      + '<div class="vs-meta"><a class="vs-model" href="' + c.href + '">' + c.model + '</a>'
      + '<span class="dim">' + c.run + ' · ' + c.date + '</span>'
      + '<span class="dim">' + c.coaster + ' · ' + c.mode + ' · ' + c.harness + '</span></div></div>';
  }

  function bar(m, a, b) {
    var va = a[m.key], vb = b[m.key];
    // Fill is proportional so the winning side always reads as the bigger bar:
    // magnitude for higher-is-better and neutral metrics, its inverse for
    // lower-is-better ones (cost, tokens, similarity).
    var weight = function (v) {
      if (v == null) return 0;
      if (m.dir === 'down') return 1 / (Math.max(0, v) + 1e-6);
      return Math.max(0, v);
    };
    var wa = weight(va), wb = weight(vb);
    var total = wa + wb;
    var pa = total > 0 ? (wa / total) * 100 : 50;
    var win = winner(m, a, b);
    return '<div class="vs-metric' + (m.headline ? ' vs-headline' : '') + '">'
      + '<span class="vs-val vs-val-a ' + (win === 'a' ? 'vs-win' : '') + '">' + m.fmt(va) + '</span>'
      + '<span class="vs-label">' + m.label + '</span>'
      + '<span class="vs-val vs-val-b ' + (win === 'b' ? 'vs-win' : '') + '">' + m.fmt(vb) + '</span>'
      + '<div class="vs-track"><span class="vs-fill-a ' + (win === 'a' ? 'vs-win' : '') + '" style="width:' + pa + '%"></span>'
      + '<span class="vs-fill-b ' + (win === 'b' ? 'vs-win' : '') + '" style="width:' + (100 - pa) + '%"></span></div></div>';
  }

  // Overlaid per-round excitement sparkline: which model climbed faster.
  function spark(a, b) {
    var W = 320, H = 70, pad = 6;
    var maxLen = Math.max(a.round_scores.length, b.round_scores.length, 2);
    var maxY = Math.max(10, Math.max.apply(null, a.round_scores.concat(b.round_scores, [0])));
    var line = function (scores, cls) {
      if (scores.length < 2) return '';
      var pts = scores.map(function (s, i) {
        var x = pad + (W - 2 * pad) * (i / (maxLen - 1));
        var y = H - pad - (H - 2 * pad) * (s / maxY);
        return x.toFixed(1) + ',' + y.toFixed(1);
      });
      return '<polyline class="' + cls + '" points="' + pts.join(' ') + '"/>';
    };
    return '<svg class="vs-spark" viewBox="0 0 ' + W + ' ' + H + '" role="img" '
      + 'aria-label="excitement per round">' + line(a.round_scores, 'vs-spark-a')
      + line(b.round_scores, 'vs-spark-b') + '</svg>';
  }

  function verdict(a, b) {
    var sa = a.score, sb = b.score;
    if (sa == null && sb == null) return 'Neither finished a rated coaster.';
    if (sa == null) return b.model + ' wins by default — ' + a.model + ' never rated a ride.';
    if (sb == null) return a.model + ' wins by default — ' + b.model + ' never rated a ride.';
    if (sa === sb) return 'Dead heat at ' + sa.toFixed(2) + ' excitement.';
    var w = sa > sb ? a : b, l = sa > sb ? b : a;
    return w.model + ' takes it, ' + Math.max(sa, sb).toFixed(2) + ' vs ' + Math.min(sa, sb).toFixed(2)
      + ' excitement (+' + Math.abs(sa - sb).toFixed(2) + ').';
  }

  function render() {
    var a = byId[selects.a.value], b = byId[selects.b.value];
    if (!a || !b) { result.innerHTML = ''; return; }
    var mismatch = a.coaster !== b.coaster
      ? '<p class="vs-warn">Different coasters (' + a.coaster + ' vs ' + b.coaster
        + ') — ratings are not directly comparable across ride types.</p>'
      : (a.mode !== b.mode
        ? '<p class="vs-warn">Different modes (' + a.mode + ' vs ' + b.mode + ').</p>' : '');
    result.innerHTML =
      '<div class="vs-cards">' + card(a, 'a') + '<div class="vs-mid">vs</div>' + card(b, 'b') + '</div>'
      + mismatch
      + '<p class="vs-verdict">' + verdict(a, b) + '</p>'
      + '<div class="vs-metrics">' + METRICS.map(function (m) { return bar(m, a, b); }).join('') + '</div>'
      + '<div class="vs-spark-wrap"><span class="dim">excitement per round</span>' + spark(a, b) + '</div>';
    var url = new URL(window.location);
    url.searchParams.set('a', a.id);
    url.searchParams.set('b', b.id);
    history.replaceState(null, '', url);
  }

  function pick(side, id) { if (byId[id]) selects[side].value = id; }
  var params = new URLSearchParams(window.location.search);
  pick('a', params.get('a') || root.dataset.defaultA);
  pick('b', params.get('b') || root.dataset.defaultB);
  if (selects.a.value === selects.b.value && data.length > 1) {
    // Avoid opening on a model against itself.
    selects.b.selectedIndex = selects.a.selectedIndex === 0 ? 1 : 0;
  }

  selects.a.addEventListener('change', render);
  selects.b.addEventListener('change', render);
  document.getElementById('vs-swap').addEventListener('click', function () {
    var tmp = selects.a.value; selects.a.value = selects.b.value; selects.b.value = tmp; render();
  });
  render();
})();
