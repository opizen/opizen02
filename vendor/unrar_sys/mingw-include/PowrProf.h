/* UnRAR 소스는 Windows 헤더를 대문자 이름(PowrProf.h)으로 include 하지만,
   Linux의 MinGW 헤더는 소문자(powrprof.h)라 대소문자를 구분하는 파일 시스템에서 찾지 못합니다.
   Linux에서 Windows용으로 교차 빌드할 때만 쓰는 연결용 헤더입니다. */
#include <powrprof.h>
