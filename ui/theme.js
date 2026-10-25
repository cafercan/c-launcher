'use strict';
// Tema seçimi ilk boyamadan önce uygulanır: açılışta yanlış renk parlaması olmasın.
// "system" sistemin açık/koyu tercihini izler.
(function () {
  const KEY = 'theme';
  function read() {
    try { return localStorage.getItem(KEY) || 'system'; } catch { return 'system'; }
  }
  function apply(t) {
    if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
    else delete document.documentElement.dataset.theme;
  }
  window.themeChoice = read;
  window.setTheme = function (t) {
    try { localStorage.setItem(KEY, t); } catch {}
    apply(t);
  };
  apply(read());
})();
