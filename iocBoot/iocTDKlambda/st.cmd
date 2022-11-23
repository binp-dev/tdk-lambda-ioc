#!../../bin/linux-x86_64/TDKlambda

#- You may have to change TDKlambda to something else
#- everywhere it appears in this file

< envPaths

cd "${TOP}"

## Register all support components
dbLoadDatabase "dbd/TDKlambda.dbd"
TDKlambda_registerRecordDeviceDriver pdbbase

## Load record instances
dbLoadTemplate("db/records.substitution")

cd "${TOP}/iocBoot/${IOC}"
iocInit
