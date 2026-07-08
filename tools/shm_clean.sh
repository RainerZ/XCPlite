#!/bin/bash
# Clean SHM mode shared memory artefacts

./build/shmtool clean 
rm *.a2l
rm *.bin

