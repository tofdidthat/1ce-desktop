import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { ProductionTimerPopover } from './ProductionTimer';
import './styles.css';

const timerView = new URLSearchParams(window.location.search).get('view') === 'timer';
if (timerView) document.documentElement.classList.add('timer-view');
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {timerView ? <ProductionTimerPopover /> : <App />}
  </React.StrictMode>,
);
