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

# Include dbd files from all support applications:
TDKlambda_DBD += asynRegistrars.dbd
TDKlambda_DBD += asyn.dbd
TDKlambda_DBD += stream.dbd

# Add all the support libraries needed by this IOC
TDKlambda_LIBS += stream
TDKlambda_LIBS += asyn

# TDKlambda_registerRecordDeviceDriver.cpp derives from TDKlambda.dbd
TDKlambda_SRCS += TDKlambda_registerRecordDeviceDriver.cpp

# Build the main IOC entry point on workstation OSs.
TDKlambda_SRCS_DEFAULT += TDKlambdaMain.cpp
TDKlambda_SRCS_vxWorks += -nil-

# Add support from base/src/vxWorks if needed
#TDKlambda_OBJS_vxWorks += $(EPICS_BASE_BIN)/vxComLibrary

# Finally link to the EPICS Base libraries
TDKlambda_LIBS += $(EPICS_BASE_IOC_LIBS)

#===========================

include $(TOP)/configure/RULES
#----------------------------------------
#  ADD RULES AFTER THIS LINE

