pub const CRYPTO_C: &str = r#"
/* ===== Stdlib.Crypto ===== */
#include <string.h>
#include <stdlib.h>
#include <stdint.h>

/* ---- SHA-256 ---- */
typedef struct { uint32_t s[8]; uint8_t buf[64]; uint64_t len; uint32_t blen; } certo_sha256_ctx;
static const uint32_t _sha256_k[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
};
#define SHA256_ROR(x,n) (((x)>>(n))|((x)<<(32-(n))))
#define SHA256_CH(x,y,z)  (((x)&(y))^(~(x)&(z)))
#define SHA256_MAJ(x,y,z) (((x)&(y))^((x)&(z))^((y)&(z)))
#define SHA256_S0(x) (SHA256_ROR(x,2)^SHA256_ROR(x,13)^SHA256_ROR(x,22))
#define SHA256_S1(x) (SHA256_ROR(x,6)^SHA256_ROR(x,11)^SHA256_ROR(x,25))
#define SHA256_R0(x) (SHA256_ROR(x,7)^SHA256_ROR(x,18)^((x)>>3))
#define SHA256_R1(x) (SHA256_ROR(x,17)^SHA256_ROR(x,19)^((x)>>10))
static void _sha256_block(certo_sha256_ctx *c, const uint8_t *p) {
    uint32_t w[64], a,b,d,e,f,g,h,t1,t2; int i;
    uint32_t cc = c->s[2];
    for(i=0;i<16;i++) w[i]=((uint32_t)p[i*4]<<24)|((uint32_t)p[i*4+1]<<16)|((uint32_t)p[i*4+2]<<8)|p[i*4+3];
    for(i=16;i<64;i++) w[i]=SHA256_R1(w[i-2])+w[i-7]+SHA256_R0(w[i-15])+w[i-16];
    a=c->s[0];b=c->s[1];cc=c->s[2];d=c->s[3];e=c->s[4];f=c->s[5];g=c->s[6];h=c->s[7];
    for(i=0;i<64;i++){
        t1=h+SHA256_S1(e)+SHA256_CH(e,f,g)+_sha256_k[i]+w[i];
        t2=SHA256_S0(a)+SHA256_MAJ(a,b,cc);
        h=g;g=f;f=e;e=d+t1;d=cc;cc=b;b=a;a=t1+t2;
    }
    c->s[0]+=a;c->s[1]+=b;c->s[2]+=cc;c->s[3]+=d;
    c->s[4]+=e;c->s[5]+=f;c->s[6]+=g;c->s[7]+=h;
}
static void _sha256_init(certo_sha256_ctx *c) {
    c->s[0]=0x6a09e667;c->s[1]=0xbb67ae85;c->s[2]=0x3c6ef372;c->s[3]=0xa54ff53a;
    c->s[4]=0x510e527f;c->s[5]=0x9b05688c;c->s[6]=0x1f83d9ab;c->s[7]=0x5be0cd19;
    c->len=c->blen=0;
}
static void _sha256_update(certo_sha256_ctx *c, const uint8_t *d, size_t n) {
    for(size_t i=0;i<n;i++){
        c->buf[c->blen++]=d[i]; c->len++;
        if(c->blen==64){_sha256_block(c,c->buf);c->blen=0;}
    }
}
static void _sha256_final(certo_sha256_ctx *c, uint8_t *out) {
    uint64_t bits=c->len*8; c->buf[c->blen++]=0x80;
    if(c->blen>56){while(c->blen<64)c->buf[c->blen++]=0;_sha256_block(c,c->buf);c->blen=0;}
    while(c->blen<56)c->buf[c->blen++]=0;
    for(int i=7;i>=0;i--){c->buf[c->blen++]=(uint8_t)(bits>>(i*8));}
    _sha256_block(c,c->buf);
    for(int i=0;i<8;i++){out[i*4]=(uint8_t)(c->s[i]>>24);out[i*4+1]=(uint8_t)(c->s[i]>>16);out[i*4+2]=(uint8_t)(c->s[i]>>8);out[i*4+3]=(uint8_t)c->s[i];}
}
static certo_text_t certo_crypto_sha256(certo_text_t s) {
    certo_sha256_ctx c; uint8_t h[32];
    _sha256_init(&c); _sha256_update(&c,(const uint8_t*)s,strlen(s)); _sha256_final(&c,h);
    char *out=(char*)malloc(65); for(int i=0;i<32;i++)sprintf(out+i*2,"%02x",h[i]); out[64]=0;
    return out;
}

/* ---- MD5 ---- */
typedef struct { uint32_t s[4]; uint32_t c[2]; uint8_t buf[64]; } certo_md5_ctx;
static const uint32_t _md5_t[64]={
    0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,0xa8304613,0xfd469501,
    0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,0x6b901122,0xfd987193,0xa679438e,0x49b40821,
    0xf61e2562,0xc040b340,0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,
    0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,0x676f02d9,0x8d2a4c8a,
    0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,
    0x289b7ec6,0xeaa127fa,0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,
    0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,0xffeff47d,0x85845dd1,
    0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391
};
static const uint8_t _md5_s[64]={7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21};
#define MD5_ROL(x,n) (((x)<<(n))|((x)>>(32-(n))))
static void _md5_block(certo_md5_ctx *c, const uint8_t *p) {
    uint32_t w[16],a,b,cc,d,f,g,t; int i;
    for(i=0;i<16;i++) w[i]=((uint32_t)p[i*4])|((uint32_t)p[i*4+1]<<8)|((uint32_t)p[i*4+2]<<16)|((uint32_t)p[i*4+3]<<24);
    a=c->s[0];b=c->s[1];cc=c->s[2];d=c->s[3];
    for(i=0;i<64;i++){
        if(i<16){f=(b&cc)|(~b&d);g=i;}
        else if(i<32){f=(d&b)|(~d&cc);g=(5*i+1)%16;}
        else if(i<48){f=b^cc^d;g=(3*i+5)%16;}
        else{f=cc^(b|~d);g=(7*i)%16;}
        t=d;d=cc;cc=b;b=b+MD5_ROL(a+f+_md5_t[i]+w[g],_md5_s[i]);a=t;
    }
    c->s[0]+=a;c->s[1]+=b;c->s[2]+=cc;c->s[3]+=d;
}
static certo_text_t certo_crypto_md5(certo_text_t s) {
    certo_md5_ctx c; size_t len=strlen(s);
    c.s[0]=0x67452301;c.s[1]=0xefcdab89;c.s[2]=0x98badcfe;c.s[3]=0x10325476;
    c.c[0]=c.c[1]=0;
    uint8_t *padded; size_t plen=(len+8)/64*64+64; padded=(uint8_t*)calloc(plen+8,1);
    memcpy(padded,s,len); padded[len]=0x80;
    uint64_t bits=(uint64_t)len*8;
    memcpy(padded+plen-8,&bits,8);
    for(size_t i=0;i<plen;i+=64){_md5_block(&c,padded+i);}
    free(padded);
    uint8_t h[16]; for(int i=0;i<4;i++){h[i*4]=(uint8_t)c.s[i];h[i*4+1]=(uint8_t)(c.s[i]>>8);h[i*4+2]=(uint8_t)(c.s[i]>>16);h[i*4+3]=(uint8_t)(c.s[i]>>24);}
    char *out=(char*)malloc(33); for(int i=0;i<16;i++)sprintf(out+i*2,"%02x",h[i]); out[32]=0;
    return out;
}

/* ---- Base64 ---- */
static const char _b64enc[]="ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
static certo_text_t certo_crypto_base64_encode(certo_text_t s) {
    size_t len=strlen(s); size_t outlen=((len+2)/3)*4;
    char *out=(char*)malloc(outlen+1); size_t j=0;
    for(size_t i=0;i<len;i+=3){
        uint32_t v=((uint8_t)s[i]<<16)|(i+1<len?(uint8_t)s[i+1]<<8:0)|(i+2<len?(uint8_t)s[i+2]:0);
        out[j++]=_b64enc[(v>>18)&63]; out[j++]=_b64enc[(v>>12)&63];
        out[j++]=(i+1<len)?_b64enc[(v>>6)&63]:'=';
        out[j++]=(i+2<len)?_b64enc[v&63]:'=';
    }
    out[j]=0; return out;
}
static int _b64val(char c){
    if(c>='A'&&c<='Z')return c-'A';
    if(c>='a'&&c<='z')return c-'a'+26;
    if(c>='0'&&c<='9')return c-'0'+52;
    if(c=='+')return 62; if(c=='/')return 63; return -1;
}
static certo_text_t certo_crypto_base64_decode(certo_text_t s) {
    size_t len=strlen(s); char *out=(char*)malloc(len*3/4+4); size_t j=0;
    for(size_t i=0;i<len;i+=4){
        int a=_b64val(s[i]),b=_b64val(s[i+1]),c=(s[i+2]!='=')?_b64val(s[i+2]):0,d=(s[i+3]!='=')?_b64val(s[i+3]):0;
        if(a<0||b<0)break;
        out[j++]=(char)((a<<2)|(b>>4));
        if(s[i+2]!='=')out[j++]=(char)((b<<4)|(c>>2));
        if(s[i+3]!='=')out[j++]=(char)((c<<2)|d);
    }
    out[j]=0; return out;
}
"#;

pub const CRYPTO_CERTO: &str = r#"
// Stdlib.Crypto — hashing and encoding

/// SHA-256 hash of a string — returns lowercase hex (64 chars).
extern fn Crypto.sha256(s: Text): Text

/// MD5 hash of a string — returns lowercase hex (32 chars).
/// Not suitable for security-sensitive use; prefer sha256 for new code.
extern fn Crypto.md5(s: Text): Text

/// Base64-encode a string.
extern fn Crypto.base64Encode(s: Text): Text

/// Base64-decode a string. Returns empty string on invalid input.
extern fn Crypto.base64Decode(s: Text): Text
"#;
