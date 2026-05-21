PRINT "Squares and parity"

FOR I = 1 TO 8
    PRINT I; " squared is "; I * I; " ";
    IF I MOD 2 = 0 THEN
        PRINT "even"
    ELSE
        PRINT "odd"
    END IF
NEXT I

FUNCTION DOUBLEIT(N AS DOUBLE) AS DOUBLE
    DOUBLEIT = N * 2
END FUNCTION

PRINT "Double of 21 is "; DOUBLEIT(21)
