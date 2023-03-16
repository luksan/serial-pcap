# serial-cap - Record UART streams in the PCAP format

This utility can save UART streams in the PCAP format. The two Rx/Tx channels will appear as UDP
datagrams from two localhost addresses.

## Wireshark x3.28 dissector

There is a dissector written in Lua for the X3.28 serial protocol in the `wireshark` directory. 
The simplest way to load it is to start wireshark with `-Xlua_script:wireshark/x328-dissector.lua`
as a command line argument.
