/*/-============================================================================================-\*\
                                                                      ⠀⠀⡎⢉⠀⠀⠀⠀⠀⠀⠀⠀⠀
                                                                      ⠀⠀⠀⢈⣁⠆⡀⠀⠀⠀⠀⣄⠀⠀
                                                                      ⠀⠀⠀⢳⢹⠁⠀⠱⡀⠀⠀⢈⠆⠀
          ::::::::      :::     :::        ::::    ::::      :::      ⠀⠀⠀⠠⢾⣆⠀⠞⠁⠀⣠⠮⡤⡀
         :+:    :+:   :+: :+:   :+:        +:+:+: :+:+:+   :+: :+:    ⠀⠀⠀⠀⠐⠹⢦⡀⠀⠰⠁⡀⢰⡁
         +:+         +:+   +:+  +:+        +:+ +:+:+ +:+  +:+   +:+   ⠀⠀⠀⢔⠞⠛⠶⣟⣦⣀⠀⠀⠛⠀
         +#++:++#++ +#++:++#++: +#+        +#+  +:+  +#+ +#++:++#++:  ⠀⠀⠀⠌⣤⡴⠀⠀⠀⠉⠳⡰⡡⠄           
                +#+ +#+     +#+ +#+        +#+       +#+ +#+     +#+  ⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⡀⠹⡄⠀
         #+#    #+# #+#     #+# #+#        #+#       #+# #+#     #+#  ⠀⠀⠀⠀⠀⠀⠀⠀⡇⢄⠙⢀⢷⢁
          ########  ###     ### ########## ###       ### ###     ###  ⠀⠀⠀⠀⣀⣄⡀⠀⠑⠀⠀⠊⣸⢰
                                                                      ⠀⠀⠀⡔⠁⠠⠗⠀⠀⠀⠀⠀⡭⠄
                                << F O M O D   E N G I N E >>         ⠀⠀⠀⢣⠀⠀⠲⣄⣀⣀⢤⡾⠁⠀
  
\*\-============================================================================================-/*/
/*
 *               A wizardless FOMOD installer and selection-inference engine. It
 *               reads an archive's ModuleConfig, infers which options an already-
 *               installed mod was built from, and replays the install itself.
 *
 *             +--------------------------------------------------------------------+
 *
 *               Repository:   https://github.com/lextpf/salma
 *               License:      GPL
 */

import React from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'
import App from './App.tsx'
import { ThemeProvider } from './ThemeContext.tsx'
import 'material-symbols/outlined.css'
import './index.css'

const rootElement = document.getElementById('root')
if (!rootElement) {
  throw new Error('Mount point #root is missing from the page')
}

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <BrowserRouter>
      <ThemeProvider>
        <App />
      </ThemeProvider>
    </BrowserRouter>
  </React.StrictMode>,
)
