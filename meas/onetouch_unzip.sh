#!/bin/bash

for zip in $@ ; do
  unzip -p $zip > "$(basename $zip .zip).csv"
  done
