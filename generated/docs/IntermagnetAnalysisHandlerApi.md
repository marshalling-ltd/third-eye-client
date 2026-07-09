# \IntermagnetAnalysisHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_intermagnet_analysis_handler**](IntermagnetAnalysisHandlerApi.md#create_intermagnet_analysis_handler) | **POST** /api/v1/intermagnet-analysis | 
[**delete_intermagnet_analysis_handler**](IntermagnetAnalysisHandlerApi.md#delete_intermagnet_analysis_handler) | **DELETE** /api/v1/intermagnet-analysis/{id} | 
[**get_intermagnet_analysis_handler**](IntermagnetAnalysisHandlerApi.md#get_intermagnet_analysis_handler) | **GET** /api/v1/intermagnet-analysis/{id} | 
[**get_intermagnet_analysis_results_handler**](IntermagnetAnalysisHandlerApi.md#get_intermagnet_analysis_results_handler) | **GET** /api/v1/intermagnet-analysis/{id}/results | 
[**list_intermagnet_analysis_handler**](IntermagnetAnalysisHandlerApi.md#list_intermagnet_analysis_handler) | **GET** /api/v1/intermagnet-analysis | 
[**update_intermagnet_analysis_handler**](IntermagnetAnalysisHandlerApi.md#update_intermagnet_analysis_handler) | **PATCH** /api/v1/intermagnet-analysis/{id} | 



## create_intermagnet_analysis_handler

> models::IntermagnetAnalysisModel create_intermagnet_analysis_handler(create_intermagnet_analysis_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_intermagnet_analysis_schema** | [**CreateIntermagnetAnalysisSchema**](CreateIntermagnetAnalysisSchema.md) | Create intermagnet analysis | [required] |

### Return type

[**models::IntermagnetAnalysisModel**](IntermagnetAnalysisModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_intermagnet_analysis_handler

> delete_intermagnet_analysis_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Intermagnet analysis id | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_intermagnet_analysis_handler

> models::IntermagnetAnalysisModel get_intermagnet_analysis_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Intermagnet analysis id for fetching intermagnet analysis | [required] |

### Return type

[**models::IntermagnetAnalysisModel**](IntermagnetAnalysisModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_intermagnet_analysis_results_handler

> models::PagedResponseTimeSeriesIntermagnetDataModel get_intermagnet_analysis_results_handler(id, start, limit, agg_level, normalize, differentiate)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Intermagnet analysis id | [required] |
**start** | Option<**String**> | The start time for the search. If not specified, it defaults to the earliest available time in the data. |  |
**limit** | Option<**i32**> | Maximum number of records to return. Defaults to 10000. |  |
**agg_level** | Option<[**TimeBucket**](.md)> | Level of data aggregation. Default is per minute. |  |
**normalize** | Option<**bool**> | If set to true, the variables will be centered to have a zero mean. Defaults to false. |  |
**differentiate** | Option<**bool**> | If set to true, the time series will be differenced, transforming each value to the change from its previous value (i.e., x_t - x_(t-1)). Defaults to false. |  |

### Return type

[**models::PagedResponseTimeSeriesIntermagnetDataModel**](PagedResponseTimeSeries_IntermagnetDataModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_intermagnet_analysis_handler

> models::PagedResponseIntermagnetAnalysisModel list_intermagnet_analysis_handler(page, limit, order_by, desc, name, r#type, status)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**name** | Option<**String**> | intermagnet analysis name |  |
**r#type** | Option<[**IntermagnetAnalysisType**](.md)> | analysis type |  |
**status** | Option<[**IntermagnetAnalysisStatus**](.md)> | analysis status |  |

### Return type

[**models::PagedResponseIntermagnetAnalysisModel**](PagedResponse_IntermagnetAnalysisModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## update_intermagnet_analysis_handler

> models::IntermagnetAnalysisModel update_intermagnet_analysis_handler(id, update_intermagnet_analysis_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Intermagnet analysis id for fetching intermagnet analysis | [required] |
**update_intermagnet_analysis_schema** | [**UpdateIntermagnetAnalysisSchema**](UpdateIntermagnetAnalysisSchema.md) | Update intermagnet analysis | [required] |

### Return type

[**models::IntermagnetAnalysisModel**](IntermagnetAnalysisModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

