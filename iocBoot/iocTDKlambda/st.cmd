#!../../bin/linux-x86_64/TDKlambda

#- You may have to change TDKlambda to something else
#- everywhere it appears in this file

< envPaths

cd "${TOP}"

## Register all support components
dbLoadDatabase "dbd/TDKlambda.dbd"
TDKlambda_registerRecordDeviceDriver pdbbase

## Config stream protocol
epicsEnvSet("STREAM_PROTOCOL_PATH", "${TOP}/protocols")

#drvAsynIPPortConfigure ("PS1", "127.0.0.1:10000")

drvAsynSerialPortConfigure ("PS1","/dev/ttyUSB0")
asynSetOption ("PS1", 0, "baud", "19200")
asynSetOption ("PS1", 0, "bits", "8")
asynSetOption ("PS1", 0, "parity", "none")
asynSetOption ("PS1", 0, "stop", "1")
asynSetOption ("PS1", 0, "clocal", "N")
asynSetOption ("PS1", 0, "crtscts", "N")
asynSetOption ("PS1", 0, "ixon", "N")
asynSetOption ("PS1", 0, "ixany", "N")

## Load record instances
dbLoadTemplate("db/fast_records.substitution")
dbLoadTemplate("db/records.substitution")

cd "${TOP}/iocBoot/${IOC}"
iocInit

## Start any sequence programs
#seq sncxxx,"user=dima"
