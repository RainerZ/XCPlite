# no_a2l_demo Demo

Demonstrates XCPlite usage without runtime on-target A2L database generation.  
No file system support needed.  

This is still work in progress, intended to support RTOS like microcontroller operating systems.  
Not stable yet.  

The A2L database creator and ELF/DWARF reader in the xcpclient tool are part of the solution to replace the on-target A2L generation. 
The rust crate for the xcpclient tool can now be found in the XCPlite repository tools folder.  
See comments in main.c.  


## The XCPlite Build Time A2L Generation Concept
  
The fundamental idea is to provide a specialized A2L database creator (ELF -> A2L converter) designed exclusively for XCPlite.  
The XCPlite A2L creator knows implementation details of the XCPlite code instrumentation library to automate the A2L generation process as much as possible.  
It automatically detects all events and calibration memory segments created by the XCPlite instrumentation macros.
It detects the code locations of the event trigger points and automatically associates local and member variables with complex types existing in each events scope.  
The test XCP client in the xcpclient tool can work with the ELF file directly, no need for a separate A2L file. 


### Concept of the xcpclient A2L Creator

An XCPlite specific A2L creator/writer with ELF/DWARF reader is built into the xcpclient tool.  

Step 1: A2L template generation:

- Creates a complete A2L template with IF_DATA, memory segments and events from ELF by detecting static segment and event marker variables created by the XCPlite code instrumentaion


Step 2: A2L content generation:

- Add  calibration parameters
    The reference pages of all calibration parameters must be in addressable (4 GB - 32bit) global memory (.bss segment must be in this range)
    Parameters must be in a structure, a calibration segment contains a single structure to assure a defined memory area and layout
    Detect calibration parameters by the address of their reference page by naming convention and segment marker variable
    XCP needs to be configure for absolute calibration segment addressing
- Add measurement variables
    Global or static measurements must be in addressable (4 GB - 32bit) global memory (.bss segment must be in this range)
    Takes all global, static and local variables into account in specified compilation units
    Try to detect an appropriate fixed event for each variable by detecting a event trigger in the same function, if not use the unsafe standard async event as default event
- Add all types required as TYPEDEF_STRUCTURE

Content generation step 2 can alternatively be done by hand, with any other A2L tool from Vector or open source

Step 3: Optional A2L Fix:

- There may be still unknown event and segment numbers, when events are created dynamically. The A2L file is consistent and complete, but the numbers do not match the runtime numbers
  This can be fixed by connecting to the target and querying segments and events via XCP
  CANape already does this for events, but not for segments


### TODO List and open issues

- The xcpclient tool is just proof of concept yet
    Make it more flexible by adding a regular expression filter to the xcpclient tool, allow to specify a list of compilation units, ...
- Support relative calibration segment addressing
    XCPlite is configured for relative calibration segment addressing as default, this needs to be changed to absolute addressing for the A2L creator to work
    As long as all reference pages are in a 4 GB addressable range, there is no benefit of relative addressing
    By convention, parameters always use address extension 0 ACFDD or CAFDD
    Currently we preliminary use AAFDD, because the XCPlite macros for event triggering so not detect the calibration segment addressing mode and CANape can not handle ACFDD
- Free standing parameters not in calibration segments
    Not implemented yet, would probably need code macro annotations to detect them
- Make sure the event trigger location and the variable location have the same CFA (have not seen any violations yet)
- Heap measurement variables
    The A2L creator can not handle heap variables yet
    Needs to detect trg__AAS or trg__AASD type and analyze the argument type of DaqTriggerEvent(), pointer to type
- Thread local variables
    The A2L creator can not handle thread local variables yet
    The DAQ capture method does not work for TLS, need a ApplXcpGetTlsBaseAddress() function, maybe introduce AAST type
    Detect the base address of the TLS block, like it is done in ApplXcpGetBaseAddr()/xcp_get_base_addr() for the global variables
    The DaqCapture macros as an alternative, does not work yet
- EPK
    Detect if the target application has a EPK segment or not
    Currently no EPK segment is generated, switched off in XCPlite
- Function parameters
    Define a macro to declare function parameters as XCP_MEA, which spills them to stack
    A2L Creator ELF reader parser must detect the function parameters with the CFA offset in the stack frame
- bool
    The A2L creator must create a BOOL conversion rule and detect the size of the bool type
- Support for C++, name spaces, classes, member functions, ...

## Using the xcpclient tool for A2L generation

Example: Create an A2L template from target:  
(Note that the tool will connect to the target ECU to get event id and calibration memory segment number information)

```bash
$xcpclient   --dest-addr=$TARGET_HOST:5555 --tcp --create-a2l --a2l no_a2l_demo.a2l
```

Example: Create an A2L template from binary file with debug information:  
(Note that the protocol information is only needed, to store it in the A2L file, the A2L file will be consistent, but the event ids and calibration memory segment numbers may be wrong!)

```bash
$xcpclient   --dest-addr=$TARGET_HOST:5555 --tcp --offline --elf no_a2l_demo --create-a2l --a2l no_a2l_demo.a2l
```

Example: Create measurement and calibration variables from a binary file with ELF/DWARF debug information and add variable from specified compilation units filtered by a regular expression

```bash
$xcpclient   --dest-addr=$TARGET_HOST:5555 --tcp --offline --elf no_a2l_demo  --a2l no_a2l_demo.a2l
```

Refer to the xcpclient command line parameter description for details.  

The A2L file template can now be modified, extended and updated with CANape or any other A2L tool.
This approach works for global measurement variables and calibration parameters.  
XCPlite must be configured for absolute calibration segment addressing (OPTION_CAL_SEGMENTS_ABS).  

To make local variables visible, they have to live on stack which is usually not true with optimized code.
There is a macro XCP_MEA to spill local variables to stack with minimal runtime overhead.

Example:

```bash
XCP_MEA uint8_t counter = 0;
```

.

## Other A2L generation options

### Using Vector CANape

Drop the template generated by xcpclient into CANape and create a new XCP on Ethernet device.  
Enable access to the ELF file in the device configuration.  
Use the A2L editor to add individual measurement parameters.  
For calibration create an calibration instance for the complete calibration parameter struct.

### Using Vector A2L-Toolset A2L-Creator to add measurement and calibration metadata

The example code contains some A2L creator metadata annotation to add metadata such as calibration variable limits and physical units

### Using Open Source a2ltool

Example:
Add the calibration segment 'params' and the measurement variable 'counter' to the A2L template:

```bash
a2ltool  --update --measurement-regex "counter"  --characteristic-regex "params" --elffile  no_a2l_demo  --enable-structures --output no_a2l_demo no_a2l_demo 
```


