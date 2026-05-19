rm build/CMakeCache.txt
cmake --build build --target clean
cmake -B build -S . -DCMAKE_BUILD_TYPE=Debug -DCMAKE_C_FLAGS="-DXCPLIB_NO_A2L"
cmake --build build --target no_a2l_demo
