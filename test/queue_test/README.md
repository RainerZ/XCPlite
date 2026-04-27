
# Queue test Queue64v

```

// Test parameters
// 64 byte payload  * THREAD_COUNT * 1000000/THREAD_DELAY_US = Throughput in byte/s

// Parameters for 2000000 msg/s with 10 threads, 64 byte payload, 10us delay
#define THREAD_COUNT 10                            // Number of threads to create
#define MAX_PRODUCERS 8                            // Max concurrent producer processes (SHM mode); also bounds last_counter[] in single-process mode
#define THREAD_DELAY_US 10                         // Delay in microseconds for the thread loops
#define THREAD_BURST_SIZE 2                        // Acquire and push this many entries in a burst before sleeping
#define THREAD_PAYLOAD_SIZE (4 * sizeof(uint64_t)) // Size of the test payload produced by the threads


On Raspberry Pi 5:

Producer acquire lock time statistics:
  count=6210636  max_spins=0  max=34407ns  avg=181ns

Lock time histogram (6210636 events):
  Range                      Count        %  Bar
  --------------------  ----------  -------  ------------------------------
  80-120ns                  677202   10.90%  ######
  120-160ns                3041621   48.97%  ##############################
  160-200ns                 274267    4.42%  ##
  200-240ns                 166816    2.69%  #
  240-280ns                1651532   26.59%  ################
  280-320ns                 252647    4.07%  ##
  320-360ns                  78341    1.26%  
  360-400ns                  33250    0.54%  
  400-600ns                  33159    0.53%  
  600-800ns                    882    0.01%  
  800-1000ns                    82    0.00%  
  1000-1500ns                   21    0.00%  
  1500-2000ns                   74    0.00%  
  2000-3000ns                  424    0.01%  
  3000-4000ns                  180    0.00%  
  4000-6000ns                  105    0.00%  
  6000-8000ns                   20    0.00%  
  8000-10000ns                   2    0.00%  
  10000-20000ns                  6    0.00%  
  20000-40000ns                  5    0.00%  



On Mac OS:

Producer acquire lock time statistics:
  count=535761606  max=18446744073709551536ns  avg=47ns (cal=39ns)

Lock time histogram (535761606 events):
  Range                      Count        %  Bar
  --------------------  ----------  -------  ------------------------------
  0-10ns                 275278228   51.38%  ##############################
  10-20ns                        0    0.00%  
  20-40ns                        0    0.00%  
  40-80ns                161463875   30.14%  #################
  80-120ns                77886692   14.54%  ########
  120-160ns                7696017    1.44%  
  160-200ns                3574859    0.67%  
  200-300ns                6259021    1.17%  
  300-400ns                 682693    0.13%  
  400-500ns                 231105    0.04%  
  500-600ns                 242962    0.05%  
  600-800ns                 505533    0.09%  
  800-1000ns                369412    0.07%  
  1000-1500ns               597985    0.11%  
  1500-2000ns               252219    0.05%  
  2000-3000ns               206568    0.04%  
  3000-4000ns                85610    0.02%  
  4000-6000ns                92120    0.02%  
  6000-8000ns                49565    0.01%  
  8000-10000ns               78214    0.01%  
  10000-20000ns             179376    0.03%  
  20000-40000ns              26996    0.01%  
  40000-80000ns               2431    0.00%  
  80000-160000ns                93    0.00%  
  160000-320000ns                5    0.00%  
  >320000ns                     27    0.00%  



```



# Queue test Queue64f

```
On Mac OS:

Producer acquire lock time statistics:
  count=17331306  max=90680ns  avg=35ns (cal=28ns)

Lock time histogram (17331306 events):
  Range                      Count        %  Bar
  --------------------  ----------  -------  ------------------------------
  0-10ns                   2967660   17.12%  #########
  10-20ns                  9093855   52.47%  ##############################
  20-40ns                        0    0.00%  
  40-80ns                  4344064   25.06%  ##############
  80-120ns                  670425    3.87%  ##
  120-160ns                 102755    0.59%  
  160-200ns                  36082    0.21%  
  200-300ns                  35659    0.21%  
  300-400ns                  15091    0.09%  
  400-500ns                   4879    0.03%  
  500-600ns                   7083    0.04%  
  600-800ns                  10778    0.06%  
  800-1000ns                 12134    0.07%  
  1000-1500ns                13760    0.08%  
  1500-2000ns                 5077    0.03%  
  2000-3000ns                 3506    0.02%  
  3000-4000ns                 1210    0.01%  
  4000-6000ns                 1771    0.01%  
  6000-8000ns                  758    0.00%  
  8000-10000ns                1034    0.01%  
  10000-20000ns               2667    0.02%  
  20000-40000ns                949    0.01%  
  40000-80000ns                104    0.00%  
  80000-160000ns                 5    0.00%  
  160000-320000ns                0    0.00%  
  >320000ns                      0    0.00%  


```