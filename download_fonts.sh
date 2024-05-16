#!/bin/bash
mkdir -p fonts
cd fonts
if [ ! -d "source-sans" ]; then
    curl -L "https://github.com/adobe-fonts/source-sans/releases/download/3.052R/VF-source-sans-3.052R.zip" > source-sans.zip
    7z e source-sans.zip -osource-sans *.ttf -r
    mv source-sans/*-Upright.ttf source-sans/upright.ttf
    mv source-sans/*-Italic.ttf  source-sans/italic.ttf
    rm source-sans.zip
fi

if [ ! -d "crimson-pro" ]; then
    mkdir -p crimson-pro
    curl -L "https://github.com/Fonthausen/CrimsonPro/raw/master/fonts/variable/CrimsonPro-Italic%5Bwght%5D.ttf" > crimson-pro/italic.ttf
    curl -L "https://github.com/Fonthausen/CrimsonPro/raw/master/fonts/variable/CrimsonPro%5Bwght%5D.ttf" > crimson-pro/upright.ttf
fi
