// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Client-side search over SEARCH_INDEX loaded from search-index.js.

(function () {
  const input = document.getElementById('search');
  const results = document.getElementById('search-results');
  if (!input || !results) return;

  function search(q) {
    if (typeof SEARCH_INDEX === 'undefined') return [];
    const lower = q.toLowerCase();
    return SEARCH_INDEX.filter(function (e) {
      return e.name.toLowerCase().includes(lower);
    }).slice(0, 10);
  }

  function render(matches) {
    results.innerHTML = matches
      .map(function (e) {
        return (
          '<a href="' + e.url + '">' +
          e.name +
          '<span class="search-kind">' + e.kind + '</span>' +
          '</a>'
        );
      })
      .join('');
    results.hidden = matches.length === 0;
  }

  input.addEventListener('input', function () {
    const q = this.value.trim();
    if (!q) {
      results.hidden = true;
      return;
    }
    render(search(q));
  });

  document.addEventListener('click', function (e) {
    if (!results.contains(e.target) && e.target !== input) {
      results.hidden = true;
    }
  });

  input.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      results.hidden = true;
      input.blur();
    }
  });
})();
