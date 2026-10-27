'use strict';

// Chamadas pelo Rust (overlay.rs) com webview.eval.
const card = document.getElementById('card');

window.showCard = ({ title, subtitle, kind }) => {
  document.getElementById('title').textContent = title;
  document.getElementById('subtitle').textContent = subtitle;
  card.classList.toggle('error', kind === 'error');
  card.classList.remove('show');
  // Força o recomeço da animação quando um card chega com outro ainda na tela.
  void card.offsetWidth;
  card.classList.add('show');
};

window.hideCard = () => card.classList.remove('show');
