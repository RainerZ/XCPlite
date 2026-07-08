#!/bin/bash
# Get status of SHM mode applications

../../build-shm/shmtool status -v

bintool --bin main.bin --dump --verbose 
