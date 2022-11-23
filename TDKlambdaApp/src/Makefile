TOP=../..

include $(TOP)/configure/CONFIG
#----------------------------------------
#  ADD MACRO DEFINITIONS AFTER THIS LINE
#=============================

#=============================
# Build the IOC application

PROD_IOC = TDKlambda
# TDKlambda.dbd will be created and installed
DBD += TDKlambda.dbd

# TDKlambda.dbd will be made up from these files:
TDKlambda_DBD += base.dbd

# TDKlambda_registerRecordDeviceDriver.cpp derives from TDKlambda.dbd
TDKlambda_SRCS += TDKlambda_registerRecordDeviceDriver.cpp

# Build the main IOC entry point.
TDKlambda_SRCS += TDKlambdaMain.cpp

# Use Ferrite
TDKlambda_DBD += Ferrite.dbd
TDKlambda_LIBS += Ferrite
SHRLIB_SEARCH_DIRS += $(TOP)/target/debug
TDKlambda_LIBS += backend

# Finally link to the EPICS Base libraries
TDKlambda_LIBS += $(EPICS_BASE_IOC_LIBS)

#===========================

include $(TOP)/configure/RULES
#----------------------------------------
#  ADD RULES AFTER THIS LINE

