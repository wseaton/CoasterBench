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
