#!/bin/bash
mkdir -p "fonts"
cd fonts
if [ ! -d "source-sans" ]; then
    curl -L "https://github.com/adobe-fonts/source-sans/releases/download/3.052R/VF-source-sans-3.052R.zip" > source-sans.zip
    7z e source-sans.zip -osource-sans *.otf -r
    mv source-sans/*-Upright.otf source-sans/upright.otf
    mv source-sans/*-Italic.otf  source-sans/italic.otf
    rm source-sans.zip
fi
